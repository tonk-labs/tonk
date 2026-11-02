use axum::{Json, extract::State};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject as UcanSubject};
use dialog_varsig::Principal as _;
use tonk_invite::connection::AgentAccess;
use tonk_invite::{Invite, InviteAudience};
use tonk_schema::agent::AgentSpace;
use zeroize::Zeroizing;

use super::*;
use crate::router::{agents, repository};

/// An open share link to `space`, as a member mints one.
async fn share_link(space: &Ed25519Signer) -> String {
    let seed: [u8; 32] = rand::random();
    let link_key = Ed25519Signer::import(&seed).await.unwrap();
    let delegation = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&link_key.did())
        .subject(UcanSubject::Specific(space.did()))
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    Invite::new(
        DelegationChain::new(delegation),
        InviteAudience::Open { seed },
        None,
    )
    .await
    .unwrap()
    .with_space_name(Some("Family trip".into()))
    .to_url("https://tonk.network/join")
    .unwrap()
}

/// A signed-in account with an agent space, as the approval page leaves
/// it. Returns the state, the account's root key, and the agent.
async fn account_with_agent() -> (AppState, Ed25519Signer, Did, String) {
    let state = crate::router::command::tests::native::test_state().await;
    // The account root, linked to this device as signing in links it.
    let root = Ed25519Signer::generate().await.unwrap();
    {
        let tonk = state.read().await;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .unwrap();
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "test-credential".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .unwrap();
    }
    let seed = Zeroizing::new(rand::random::<[u8; 32]>());
    let agent = Ed25519Signer::import(&*seed).await.unwrap().did();
    let key = repository::create_space_from_seed(&state, "Claude", None, seed)
        .await
        .unwrap();
    {
        let tonk = state.read().await;
        tonk.reactor
            .repository(&key)
            .branch(repository::CONTENT_BRANCH)
            .transaction()
            .assert(AgentSpace::new(agent.to_string().parse().unwrap()))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }
    (state, root, agent, key)
}

/// The account claims a share link, then signs the agent's grant with
/// its own key: the chain runs through the account, "your agents" lists
/// it as the account's, and the account can sign the revocation the
/// space's access service takes.
#[dialog_common::test]
async fn it_grants_a_joined_space_through_the_account_and_can_revoke_it() -> anyhow::Result<()> {
    let (state, root, agent, agent_key) = account_with_agent().await;
    let shared = Ed25519Signer::generate().await?;
    let link = share_link(&shared).await;
    {
        let tonk = state.write().await;
        crate::router::join::join_invite(&tonk, &link)
            .await
            .map_err(|failure| anyhow::anyhow!("join: {failure:?}"))?;
    }

    let target = Target {
        subject: shared.did(),
        access: AgentAccess::Read,
        remote: "https://sync.example.test/ucan/".parse()?,
        label: "Family trip".into(),
    };
    let chains = {
        let tonk = state.read().await;
        issue(&tonk, &Signer::from(root.clone()), &agent, &target).await?
    };
    assert!(!chains.is_empty());
    for chain in &chains {
        let certificates: Vec<_> = chain.proofs().collect();
        let leaf = certificates.last().unwrap();
        assert_eq!(leaf.issuer(), &root.did(), "the account signs the grant");
        assert_eq!(leaf.audience(), &agent);
        assert_eq!(
            certificates[certificates.len() - 2].audience(),
            &root.did(),
            "the account holds the link's authority it passes on"
        );
        assert!(chain.ability().starts_with("/use/get"), "read only");
    }

    let Json(listed) = agents::list(State(state.clone())).await?;
    let agent_entry = listed
        .iter()
        .find(|entry| entry.did() == &agent)
        .expect("the agent is listed");
    assert_eq!(
        agent_entry.access_to(&shared.did()),
        Some("read"),
        "the joined space is listed at the access chosen"
    );

    let tonk = state.read().await;
    let profile = tonk.profile.did();
    let grants = agents::grants(&tonk, &agent).await?;
    assert!(!grants.is_empty());
    for (leaf, _, issued) in grants {
        assert_eq!(
            issued,
            agents::Issued::Here,
            "the account issued it, so it revokes it at the service"
        );
        let target = *leaf.proof_cids().last().unwrap();
        let path = agents::ancestor(&tonk, &profile, &shared.did(), &leaf)
            .await?
            .push(leaf.proofs().last().unwrap().clone())?;
        let artifact = super::super::agent_connections::revocation(&tonk, &path, &target).await?;
        let verified = tonk_identity::revocation::verify(&artifact).await?;
        assert_eq!(verified.target_cid, target.to_string());
        assert_eq!(
            verified.subject,
            root.did(),
            "the revocation exercises the account's authority"
        );
    }
    drop(tonk);
    let _ = agent_key;
    Ok(())
}

/// A link that grants only reading cannot let the agent write.
#[dialog_common::test]
async fn it_refuses_write_for_a_read_only_link() -> anyhow::Result<()> {
    let (state, root, agent, _) = account_with_agent().await;
    let shared = Ed25519Signer::generate().await?;
    let seed: [u8; 32] = rand::random();
    let link_key = Ed25519Signer::import(&seed).await?;
    let delegation = DelegationBuilder::new()
        .issuer(Signer::from(shared.clone()))
        .audience(&link_key.did())
        .subject(UcanSubject::Specific(shared.did()))
        .command(vec!["use".into(), "get".into()])
        .try_build()
        .await?;
    let link = Invite::new(
        DelegationChain::new(delegation),
        InviteAudience::Open { seed },
        None,
    )
    .await?
    .to_url("https://tonk.network/join")?;
    {
        let tonk = state.write().await;
        crate::router::join::join_invite(&tonk, &link)
            .await
            .map_err(|failure| anyhow::anyhow!("join: {failure:?}"))?;
    }
    let target = Target {
        subject: shared.did(),
        access: AgentAccess::ReadWrite,
        remote: "https://sync.example.test/ucan/".parse()?,
        label: "Read only".into(),
    };
    let tonk = state.read().await;
    let refused = issue(&tonk, &Signer::from(root), &agent, &target).await;
    assert!(
        matches!(refused, Err(TonkWorkerError::Forbidden(_))),
        "{refused:?}"
    );
    Ok(())
}
