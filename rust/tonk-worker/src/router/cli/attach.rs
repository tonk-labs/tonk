//! Explicit peer upstream selection. An inventory row is not a membership
//! record and this path never provisions a consumer or creates a delegation.

use dialog_capability::{Provider, Subject};
use dialog_effects::memory::prelude::*;
use dialog_iroh_remote::site::IrohAddress;
use dialog_repository::{RemoteAddress, Repository, RepositoryExt, SiteAddress};
use dialog_varsig::{Did, Principal};

use super::super::repository::{BranchConfiguration, RemoteConfiguration, RepositoryConfiguration};

/// Stable across route/certificate changes, distinct for each CLI identity.
fn remote_name(peer: &IrohAddress) -> String {
    format!(
        "peer-{}",
        peer.did().to_string().trim_start_matches("did:key:")
    )
}

fn configuration(peer: IrohAddress, subject: Did) -> RepositoryConfiguration {
    let name = remote_name(&peer);
    RepositoryConfiguration::default()
        .remote(
            &name,
            RemoteConfiguration::new(SiteAddress::Iroh(peer)).subject(subject),
        )
        .branch(
            "main",
            BranchConfiguration::default().upstream(&name, "main"),
        )
}

/// Change only a route for the SAME endpoint and subject. Resolve + publish
/// uses the ordinary cell CAS; a concurrent edit is an error, not an overwrite.
/// Kept separate from `ensure_remote_config`, whose general-purpose contract
/// is to preserve an existing remote, even when a caller supplies a new address.
async fn refresh_route<C, Env>(
    repository: &Repository<C>,
    peer: &IrohAddress,
    env: &Env,
) -> Result<(), String>
where
    C: Principal + Clone,
    Env: Provider<dialog_effects::memory::Resolve> + Provider<dialog_effects::memory::Publish>,
{
    let cell = repository.remote(remote_name(peer)).address();
    cell.resolve()
        .perform(env)
        .await
        .map_err(|e| e.to_string())?;
    let Some(existing) = cell.content() else {
        return Ok(()); // The normal configuration path creates it.
    };
    if existing.subject() != &repository.did()
        || !matches!(existing.site(), SiteAddress::Iroh(old) if old.did() == peer.did())
    {
        return Err("the peer remote name is already used by a different identity or space".into());
    }
    let address = RemoteAddress::new(SiteAddress::Iroh(peer.clone()), repository.did());
    if existing != address {
        cell.publish(address)
            .perform(env)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Resolve only against the latest successful observation. A stale click after
/// switching peers must not attach the replacement CLI to the old offer.
fn offered(owner: &super::Lazy, uri: &str, subject: &str) -> Option<usize> {
    let offers = owner.offers.lock().expect("peer offers lock");
    let (address, spaces) = offers.as_ref()?;
    (address == uri)
        .then(|| spaces.iter().position(|space| space.subject == subject))
        .flatten()
}

async fn attach(tonk: &crate::worker::TonkState, uri: &str, subject: &str) -> Result<(), String> {
    let (peer, _) = super::peer_route(uri)?;
    let subject: Did = subject
        .parse()
        .map_err(|_| "the offered space has an invalid DID")?;
    if super::super::account_state::is_account_key(tonk, subject.as_str()).await {
        return Err("the account repository cannot be attached as a user space".into());
    }
    let local = super::super::join::find_replica_for_subject(tonk, &subject)
        .await
        .map_err(|e| e.to_string())?;
    // Use the existing certificate-store walk, not the operator's broad local
    // storage permission. This is only a local preflight: the CLI still verifies
    // each actual invocation, including expiry and current/cached revocation.
    // `meta` contains this device's replica/remotes/tracking configuration.
    // It stays local, just as it does for an ordinary cloud remote.
    for branch in ["main"] {
        let read = Subject::from(subject.clone())
            .memory()
            .space(format!("branch/{branch}"))
            .cell("revision")
            .resolve();
        tonk.profile
            .access()
            .claim(read)
            .perform(&tonk.operator)
            .await
            .map_err(|_| "access needed: this profile has no current authority for the space")?;
    }
    // A recovered account may already have authority and a directory entry
    // without a local replica. Only AFTER proving that authority, reuse normal
    // directory adoption. It makes a verifier-only replica, preserves cloud
    // configuration, and does not provision a consumer or mint a delegation.
    if !local
        && !super::super::adopt::ensure_space_mounted(tonk, subject.as_str())
            .await
            .map_err(|e| e.to_string())?
    {
        return Err("access needed: join or recover this space on this profile first".into());
    }
    let repository = tonk
        .profile
        .repository(subject.as_str())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| e.to_string())?;
    if repository.did() != subject {
        return Err("the local replica does not match the offered space".into());
    }
    let _mutation = tonk.admission.mutation(subject.as_str());
    refresh_route(&repository, &peer, &tonk.operator).await?;
    let config = configuration(peer, subject.clone());
    let effective = super::super::repository::ensure_remote_config(
        tonk,
        &repository,
        subject.as_str(),
        &config,
    )
    .await
    .map_err(|e| e.to_string())?;
    // Uses the existing mount-directory convention; does not erase the cloud
    // remote, its revocation relay, or any of the space's authority records.
    super::super::repository::try_record_space_mount(tonk, &subject, &effective, None)
        .await
        .map_err(|error| {
            format!("peer configured locally, but saving the mount failed; retry: {error}")
        })?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue
        .mark_dirty(subject.as_str(), js_sys::Date::now());
    Ok(())
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Provider<tonk_schema::command::AttachPeer> for crate::router::CommandEnv {
    async fn execute(&self, command: tonk_schema::command::AttachPeer) {
        // Also enforce this for direct provider callers, not only the registry.
        if !self.from_profile() {
            return;
        }
        // Serialize configuration with profile changes and inventory replacement.
        // No network operation occurs while this guard is held.
        let tonk = self.state().write().await;
        let Some(index) = offered(&tonk.reach, &command.peer.0, &command.space.0) else {
            return;
        };
        let result = attach(&tonk, &command.peer.0, &command.space.0).await;
        let detail = match result {
            Ok(()) => "peer upstream selected; sync uses your existing authority".to_owned(),
            Err(detail) => detail,
        };
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        if let Ok(session) = tonk
            .reactor
            .profile_repository()
            .branch(super::PROFILE_BRANCH)
            .acquire(&tonk.operator)
            .await
        {
            session
                .state
                .assert_overlay(tonk_schema::peer::PeerOfferDetail {
                    this: format!("state:cli/offer/{index}")
                        .parse()
                        .expect("offer entity"),
                    detail: tonk_schema::domain::peer_offer::Detail(detail),
                });
            tonk.reactor
                .schedule_poll(std::sync::Arc::clone(&session.state));
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let _ = (index, detail);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_selects_content_and_names_the_identity_not_the_route() {
        let (peer, _) =
            super::super::peer_route(&super::super::tests::uri("127.0.0.1", 40001)).unwrap();
        let (moved, _) =
            super::super::peer_route(&super::super::tests::uri("127.0.0.1", 40002)).unwrap();
        assert_eq!(remote_name(&peer), remote_name(&moved));
        let subject = peer.did();
        let config = configuration(peer.clone(), subject.clone());
        assert_eq!(config.remote.len(), 1);
        assert!(!config.remote.contains_key("origin"));
        assert_eq!(
            config.remote[&remote_name(&peer)].subject.as_ref(),
            Some(&subject)
        );
        assert!(config.remote[&remote_name(&peer)].revocation_url.is_none());
        assert!(
            !config.branch.contains_key("meta"),
            "device-local metadata must not replicate"
        );
        for branch in ["main"] {
            let upstream = config.branch[branch].upstream.as_ref().unwrap();
            assert_eq!(upstream.remote, remote_name(&peer));
            assert_eq!(upstream.branch, branch);
        }
    }

    #[test]
    fn inventory_selection_rejects_stale_routes_and_undisclosed_subjects() {
        let owner = super::super::Lazy::default();
        assert_eq!(offered(&owner, "peer-a", "space-a"), None);
        *owner.offers.lock().unwrap() = Some((
            "peer-a".into(),
            vec![super::super::Space {
                subject: "space-a".into(),
                name: None,
            }],
        ));
        assert_eq!(offered(&owner, "peer-a", "space-a"), Some(0));
        assert_eq!(offered(&owner, "peer-b", "space-a"), None);
        assert_eq!(offered(&owner, "peer-a", "space-b"), None);
        *owner.offers.lock().unwrap() = None;
        assert_eq!(offered(&owner, "peer-a", "space-a"), None);
    }

    #[dialog_common::test]
    async fn route_refresh_preserves_cloud_and_refuses_identity_or_subject_replacement() {
        let env = dialog_storage::provider::Volatile::new();
        let repository =
            Repository::from(dialog_credentials::Ed25519Signer::generate().await.unwrap());
        let (peer, _) =
            super::super::peer_route(&super::super::tests::uri("127.0.0.1", 40001)).unwrap();
        let (moved, _) =
            super::super::peer_route(&super::super::tests::uri("127.0.0.1", 40002)).unwrap();
        let name = remote_name(&peer);
        let cloud = SiteAddress::from(dialog_remote_ucan_s3::UcanAddress::new(
            "https://cloud.example/ucan/",
        ));
        repository
            .remote("origin")
            .create(cloud.clone())
            .perform(&env)
            .await
            .unwrap();
        refresh_route(&repository, &peer, &env).await.unwrap();
        assert!(repository.remote(&name).load().perform(&env).await.is_err());
        repository
            .remote(&name)
            .create(SiteAddress::Iroh(peer.clone()))
            .perform(&env)
            .await
            .unwrap();
        refresh_route(&repository, &moved, &env).await.unwrap();
        let saved = repository.remote(&name).load().perform(&env).await.unwrap();
        assert_eq!(saved.address().site(), &SiteAddress::Iroh(moved.clone()));
        assert_eq!(
            repository
                .remote("origin")
                .load()
                .perform(&env)
                .await
                .unwrap()
                .address()
                .site(),
            &cloud
        );
        let cell = repository.remote(&name).address();
        cell.resolve().perform(&env).await.unwrap();
        let edition = cell.edition();
        refresh_route(&repository, &moved, &env).await.unwrap();
        cell.resolve().perform(&env).await.unwrap();
        assert_eq!(
            cell.edition(),
            edition,
            "an unchanged route does not publish"
        );
        for conflict in [
            RemoteAddress::new(cloud, repository.did()),
            RemoteAddress::new(SiteAddress::Iroh(moved.clone()), peer.did()),
            RemoteAddress::new(
                SiteAddress::Iroh(IrohAddress::new(iroh::EndpointAddr::new(
                    iroh::SecretKey::from_bytes(&[8; 32]).public(),
                ))),
                repository.did(),
            ),
        ] {
            cell.publish(conflict.clone()).perform(&env).await.unwrap();
            assert!(refresh_route(&repository, &peer, &env).await.is_err());
            cell.resolve().perform(&env).await.unwrap();
            assert_eq!(cell.content(), Some(conflict));
        }
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod browser_tests {
    use super::*;
    use crate::router::{CommandEnv, CommandOrigin};

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn authorized_directory_space_is_adopted_but_directory_alone_grants_nothing() {
        let tonk = crate::router::tests::test_state().await;
        let owner = Repository::from(dialog_credentials::Ed25519Signer::generate().await.unwrap());
        let subject = owner.did();
        let uri = super::super::tests::uri("127.0.0.1", 40001);
        let (peer, _) = super::super::peer_route(&uri).unwrap();
        let cloud = SiteAddress::from(dialog_remote_ucan_s3::UcanAddress::new(
            "https://cloud.example/ucan/",
        ));
        let cloud_config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(cloud.clone()).subject(subject.clone()),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        super::super::super::repository::try_record_space_mount(
            &tonk,
            &subject,
            &cloud_config,
            None,
        )
        .await
        .unwrap();

        assert!(
            attach(&tonk, &uri, subject.as_str())
                .await
                .unwrap_err()
                .contains("no current authority")
        );
        assert!(
            !super::super::super::join::find_replica_for_subject(&tonk, &subject)
                .await
                .unwrap()
        );
        assert!(
            tonk.profile
                .repository(subject.as_str())
                .load()
                .perform(&tonk.operator)
                .await
                .is_err(),
            "a directory row must not even create a local credential before the authority check"
        );

        let grant = owner
            .access()
            .claim(&owner)
            .delegate(tonk.profile.did())
            .perform(&tonk.operator)
            .await
            .unwrap()
            .into_chain();
        tonk.profile
            .access()
            .save(dialog_ucan::UcanDelegation(grant))
            .perform(&tonk.operator)
            .await
            .unwrap();
        attach(&tonk, &uri, subject.as_str()).await.unwrap();
        assert!(
            super::super::super::join::find_replica_for_subject(&tonk, &subject)
                .await
                .unwrap()
        );
        let replica = tonk
            .profile
            .repository(subject.as_str())
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert_eq!(replica.did(), subject);
        assert_eq!(
            replica
                .remote("origin")
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap()
                .address()
                .site(),
            &cloud
        );
        for branch in ["main"] {
            // A fresh replica has upstream cells but no content head until
            // its first pull. Inspect configuration without requiring content.
            let upstream = replica.branch(branch).upstream();
            upstream.resolve().perform(&tonk.operator).await.unwrap();
            assert!(
                matches!(upstream.content().and_then(|entries| entries.default_upstream().cloned()), Some(dialog_repository::Upstream::Remote { remote, branch: tracked, .. })
                if remote == remote_name(&peer) && tracked == branch)
            );
            assert!(
                super::super::super::offline_sync_allowed(&tonk, subject.as_str(), branch).await,
                "the first peer pull must remain eligible offline even without a local head"
            );
        }
    }

    #[dialog_common::test]
    async fn selection_preserves_cloud_authority_and_survives_reload() {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "peer-attach").await;
        let uri = super::super::tests::uri("127.0.0.1", 40001);
        let (peer, _) = super::super::peer_route(&uri).unwrap();
        let name = remote_name(&peer);
        {
            let tonk = state.read().await;
            let repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            let subject = repository.did();
            let prefix = super::super::super::repository::space_root_prefix(&tonk, &subject)
                .await
                .unwrap()
                .to_bytes()
                .unwrap();
            let cloud = SiteAddress::from(dialog_remote_ucan_s3::UcanAddress::new(
                "https://cloud.example/ucan/",
            ));
            let cloud_config = RepositoryConfiguration::default()
                .remote(
                    "origin",
                    RemoteConfiguration::new(cloud.clone())
                        .revocation_url("https://cloud.example/revocations/".parse().unwrap()),
                )
                .branch(
                    "main",
                    BranchConfiguration::default().upstream("origin", "main"),
                );
            super::super::super::repository::ensure_remote_config(
                &tonk,
                &repository,
                &key,
                &cloud_config,
            )
            .await
            .unwrap();
            super::super::super::repository::record_space_mount(
                &tonk,
                &subject,
                &cloud_config,
                None,
            )
            .await;
            attach(&tonk, &uri, subject.as_str()).await.unwrap();
            attach(&tonk, &uri, subject.as_str()).await.unwrap();
            let moved = super::super::tests::uri("127.0.0.1", 40002);
            attach(&tonk, &moved, subject.as_str()).await.unwrap();
            let repository = tonk
                .profile
                .repository(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            assert_eq!(
                repository
                    .remote("origin")
                    .load()
                    .perform(&tonk.operator)
                    .await
                    .unwrap()
                    .address()
                    .site(),
                &cloud
            );
            let saved = repository
                .remote(&name)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            assert_eq!(saved.address().subject(), &subject);
            assert_eq!(
                saved.address().site(),
                &SiteAddress::Iroh(super::super::peer_route(&moved).unwrap().0)
            );
            for branch in ["main"] {
                let loaded = repository
                    .branch(branch)
                    .load()
                    .perform(&tonk.operator)
                    .await
                    .unwrap();
                assert!(
                    matches!(loaded.upstream(), Some(dialog_repository::Upstream::Remote { remote, branch: tracked, .. })
                    if remote == name && tracked == branch)
                );
            }
            assert_eq!(
                super::super::super::repository::space_root_prefix(&tonk, &subject)
                    .await
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                prefix
            );
            // Ordinary adoption must not switch the just-selected upstream back
            // to the cloud on the next data-plane request.
            super::super::super::adopt::ensure_space_mounted(&tonk, &key)
                .await
                .unwrap();
            let loaded = repository
                .branch("main")
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            assert!(
                matches!(loaded.upstream(), Some(dialog_repository::Upstream::Remote { remote, .. }) if remote == name)
            );
        }
    }

    #[dialog_common::test]
    async fn undisclosed_unjoined_and_content_origin_requests_do_not_register_remotes() {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "peer-guard").await;
        let uri = super::super::tests::uri("127.0.0.1", 40001);
        let (peer, _) = super::super::peer_route(&uri).unwrap();
        let command = tonk_schema::command::AttachPeer {
            this: "command:attach-test".parse().unwrap(),
            peer: tonk_schema::domain::command::attach_peer::Peer(uri.clone()),
            space: tonk_schema::domain::command::attach_peer::Space(key.clone()),
            time: tonk_schema::domain::command::attach_peer::Time(1.0),
        };
        Provider::<tonk_schema::command::AttachPeer>::execute(
            &CommandEnv::new(state.clone(), CommandOrigin::default()),
            command.clone(),
        )
        .await;
        {
            let tonk = state.read().await;
            *tonk.reach.offers.lock().unwrap() = Some((
                uri.clone(),
                vec![super::super::Space {
                    subject: key.clone(),
                    name: None,
                }],
            ));
        }
        Provider::<tonk_schema::command::AttachPeer>::execute(
            &CommandEnv::new(
                state.clone(),
                CommandOrigin {
                    repo: key.clone(),
                    branch: "main".into(),
                    client: None,
                },
            ),
            command,
        )
        .await;
        let tonk = state.read().await;
        let repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert!(
            repository
                .remote(remote_name(&peer))
                .load()
                .perform(&tonk.operator)
                .await
                .is_err()
        );
        let unknown = dialog_credentials::Ed25519Signer::generate()
            .await
            .unwrap()
            .did();
        assert!(
            attach(&tonk, &uri, unknown.as_str())
                .await
                .unwrap_err()
                .contains("access needed")
        );
        assert!(
            !super::super::super::join::find_replica_for_subject(&tonk, &unknown)
                .await
                .unwrap()
        );
        // Membership alone is not authority either: a verifier-only replica
        // with no accepted account/invite proof cannot be attached by discovery.
        let replica = super::super::super::join::mount_replica_with_configuration(
            &tonk,
            &unknown,
            RepositoryConfiguration::default(),
        )
        .await
        .unwrap();
        super::super::super::repository::record_initialized_replica_in_profile(&tonk, &unknown)
            .await
            .unwrap();
        assert!(
            attach(&tonk, &uri, unknown.as_str())
                .await
                .unwrap_err()
                .contains("no current authority")
        );
        assert!(
            replica
                .remote(remote_name(&peer))
                .load()
                .perform(&tonk.operator)
                .await
                .is_err()
        );
    }
}
