//! Offline envelope checks; real remote revocation is tested by access-service.
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan::{Parameters, Scope};
use dialog_ucan_core::time::timestamp::{Duration, UNIX_EPOCH};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject, time::Timestamp};
use dialog_varsig::Principal;
use tonk_invite::{
    connection::{
        AgentInvite, DEFAULT_GRANT_TTL_SECONDS, SpaceGrantBundle, candidate_build_scopes,
        require_grant_deadline,
    },
    home_address_meta,
};
use url::Url;

fn at(seconds: u64) -> Timestamp {
    Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds)).unwrap()
}
fn now() -> Timestamp {
    at(2_000_000_000)
}
fn remote() -> Url {
    Url::parse("https://access.example/ucan/").unwrap()
}

async fn fixture(seed: [u8; 32]) -> (Vec<DelegationChain>, Vec<Scope>) {
    let owner = Signer::from(Ed25519Signer::import(&[21; 32]).await.unwrap());
    let browser = Signer::from(Ed25519Signer::import(&[22; 32]).await.unwrap());
    let recipient = Ed25519Signer::import(&seed).await.unwrap();
    let deadline = at(now().to_unix() + DEFAULT_GRANT_TTL_SECONDS);
    let parent = DelegationBuilder::new()
        .issuer(owner.clone())
        .audience(&browser.did())
        .subject(Subject::Specific(owner.did()))
        .command(vec!["use".into()])
        .expiration(deadline)
        .try_build()
        .await
        .unwrap();
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        let leaf = DelegationBuilder::new()
            .issuer(browser.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(deadline)
            .meta(home_address_meta(&remote()))
            .try_build()
            .await
            .unwrap();
        chains.push(DelegationChain::new(parent.clone()).push(leaf).unwrap());
    }
    (chains, scopes)
}

#[dialog_common::test]
async fn connection_two_imports_retain_the_same_key_and_original_grants() {
    let (chains, scopes) = fixture([23; 32]).await;
    let cids: Vec<_> = chains
        .iter()
        .map(|chain| chain.proof_cids().to_vec())
        .collect();
    let invite = AgentInvite::new([23; 32], chains, &scopes, &remote(), now())
        .await
        .unwrap();
    let url = invite.to_url("https://tonk.network/connect").unwrap();
    let parsed = Url::parse(&url).unwrap();
    assert!(parsed.query().is_none());
    assert_eq!(parsed.path(), "/connect");
    assert!(!format!("{invite:?}").contains("tonk-agent-v1="));
    for _ in 0..2 {
        let imported = AgentInvite::parse_url(&url, &scopes, &remote(), now())
            .await
            .unwrap();
        assert_eq!(imported.secret_seed(), &[23; 32]);
        assert_eq!(
            imported.grants().expires_at(),
            at(now().to_unix() + DEFAULT_GRANT_TTL_SECONDS)
        );
        assert_eq!(
            imported
                .grants()
                .chains()
                .iter()
                .map(|chain| chain.proof_cids().to_vec())
                .collect::<Vec<_>>(),
            cids
        );
    }
}

#[dialog_common::test]
async fn connection_rejects_wrong_key_subject_and_unsigned_route() {
    let (chains, scopes) = fixture([23; 32]).await;
    assert!(
        AgentInvite::new([24; 32], chains.clone(), &scopes, &remote(), now())
            .await
            .unwrap_err()
            .to_string()
            .contains("recipient_mismatch")
    );
    let other = Ed25519Signer::import(&[25; 32]).await.unwrap();
    assert!(
        AgentInvite::new(
            [23; 32],
            chains.clone(),
            &candidate_build_scopes(&other.did()),
            &remote(),
            now()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("subject_mismatch")
    );
    assert!(
        AgentInvite::new(
            [23; 32],
            chains,
            &scopes,
            &Url::parse("https://evil.example/ucan/").unwrap(),
            now()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("untrusted_route")
    );
}

#[dialog_common::test]
async fn connection_rejects_missing_duplicate_or_broadened_rights() {
    let (mut chains, scopes) = fixture([23; 32]).await;
    chains[1] = chains[0].clone();
    assert!(
        AgentInvite::new([23; 32], chains, &scopes, &remote(), now())
            .await
            .unwrap_err()
            .to_string()
            .contains("scope_mismatch")
    );
    let (chains, mut scopes) = fixture([23; 32]).await;
    scopes[0].parameters = Parameters::default();
    assert!(
        AgentInvite::new([23; 32], chains.clone(), &scopes, &remote(), now())
            .await
            .is_err()
    );
    assert!(
        AgentInvite::new([23; 32], chains[1..].to_vec(), &scopes, &remote(), now())
            .await
            .is_err()
    );
}

#[dialog_common::test]
async fn connection_rejects_expiry_and_reports_upstream_lifetime_limit() {
    let (chains, scopes) = fixture([23; 32]).await;
    let deadline = at(now().to_unix() + DEFAULT_GRANT_TTL_SECONDS);
    require_grant_deadline(&chains, deadline).unwrap();
    let error = require_grant_deadline(&chains, at(deadline.to_unix() + 1)).unwrap_err();
    assert!(error.to_string().contains(&deadline.to_unix().to_string()));
    assert!(
        AgentInvite::new(
            [23; 32],
            chains,
            &scopes,
            &remote(),
            at(deadline.to_unix() + 1)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("invalid_chain")
    );
}

#[dialog_common::test]
async fn connection_rejects_missing_secret_unknown_version_and_tampered_proof() {
    let (chains, scopes) = fixture([23; 32]).await;
    let invite = AgentInvite::new([23; 32], chains, &scopes, &remote(), now())
        .await
        .unwrap();
    let url = invite.to_url("https://tonk.network/connect").unwrap();
    for malformed in [
        "https://tonk.network/connect".to_owned(),
        url.replace("tonk-agent-v1=", "tonk-agent-v2="),
    ] {
        assert!(
            AgentInvite::parse_url(&malformed, &scopes, &remote(), now())
                .await
                .is_err()
        );
    }
    let parsed = Url::parse(&url).unwrap();
    let bytes = bs58::decode(
        parsed
            .fragment()
            .unwrap()
            .strip_prefix("tonk-agent-v1=")
            .unwrap(),
    )
    .into_vec()
    .unwrap();
    let mut envelope: ipld_core::ipld::Ipld = serde_ipld_dagcbor::from_slice(&bytes).unwrap();
    let ipld_core::ipld::Ipld::Map(ref mut map) = envelope else {
        panic!()
    };
    let ipld_core::ipld::Ipld::List(chains) = map.get_mut("grants").unwrap() else {
        panic!()
    };
    let ipld_core::ipld::Ipld::Bytes(chain) = &mut chains[0] else {
        panic!()
    };
    let address = remote().to_string();
    let index = chain
        .windows(address.len())
        .position(|part| part == address.as_bytes())
        .unwrap();
    chain[index] = b'x';
    let tampered = format!(
        "https://tonk.network/connect#tonk-agent-v1={}",
        bs58::encode(serde_ipld_dagcbor::to_vec(&envelope).unwrap()).into_string()
    );
    assert!(
        AgentInvite::parse_url(&tampered, &scopes, &remote(), now())
            .await
            .is_err()
    );
}

#[dialog_common::test]
async fn connection_cli_owned_key_validates_without_exporting_its_secret() {
    let (chains, scopes) = fixture([31; 32]).await;
    let cli = Ed25519Signer::import(&[31; 32]).await.unwrap();
    let bundle = SpaceGrantBundle::validate(chains, &cli.did(), &scopes, &remote(), now())
        .await
        .unwrap();
    assert_eq!(bundle.recipient(), &cli.did());
    assert_eq!(bundle.chains().len(), 6);
}

#[dialog_common::test]
async fn connection_rejects_child_outliving_ancestor_and_unproven_root() {
    let (chains, scopes) = fixture([23; 32]).await;
    let owner = Signer::from(Ed25519Signer::import(&[21; 32]).await.unwrap());
    let browser = Signer::from(Ed25519Signer::import(&[22; 32]).await.unwrap());
    let wrong_root = Signer::from(Ed25519Signer::import(&[41; 32]).await.unwrap());
    for (issuer, expected) in [
        (owner.clone(), "connection_expiry_limited"),
        (wrong_root, "connection_invalid_chain"),
    ] {
        let parent = DelegationBuilder::new()
            .issuer(issuer)
            .audience(&browser.did())
            .subject(Subject::Specific(owner.did()))
            .command(vec!["use".into()])
            .expiration(at(now().to_unix() + 3600))
            .try_build()
            .await
            .unwrap();
        let modified = chains
            .iter()
            .map(|chain| {
                DelegationChain::new(parent.clone())
                    .push(chain.proofs().last().unwrap().clone())
                    .unwrap()
            })
            .collect();
        let error = AgentInvite::new([23; 32], modified, &scopes, &remote(), now())
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[dialog_common::test]
async fn connection_rejects_command_escalation_beyond_ancestor() {
    let (chains, scopes) = fixture([23; 32]).await;
    let owner = Signer::from(Ed25519Signer::import(&[21; 32]).await.unwrap());
    let browser = Signer::from(Ed25519Signer::import(&[22; 32]).await.unwrap());
    let parent = DelegationBuilder::new()
        .issuer(owner.clone())
        .audience(&browser.did())
        .subject(Subject::Specific(owner.did()))
        .command(vec!["use".into(), "get".into()])
        .expiration(at(now().to_unix() + DEFAULT_GRANT_TTL_SECONDS))
        .try_build()
        .await
        .unwrap();
    let modified = chains
        .iter()
        .map(|chain| {
            DelegationChain::new(parent.clone())
                .push(chain.proofs().last().unwrap().clone())
                .unwrap()
        })
        .collect();
    let error = AgentInvite::new([23; 32], modified, &scopes, &remote(), now())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("unsupported_ancestor_scope"),
        "{error}"
    );
}

#[dialog_common::test]
async fn connection_refuses_unsafe_carriers_and_redacts_errors() {
    let (chains, scopes) = fixture([23; 32]).await;
    let invite = AgentInvite::new([23; 32], chains, &scopes, &remote(), now())
        .await
        .unwrap();
    for base in [
        "javascript:alert(1)",
        "file:///tmp/invite",
        "http://tonk.example/connect",
        "https://user:password@tonk.example/connect",
        "https://tonk.example/connect?secret=value",
    ] {
        assert!(invite.to_url(base).is_err(), "{base}");
    }
    for base in [
        "http://localhost:8080/connect",
        "http://127.0.0.1:8080/connect",
        "http://[::1]:8080/connect",
    ] {
        let url = invite.to_url(base).unwrap();
        AgentInvite::parse_url(&url, &scopes, &remote(), now())
            .await
            .unwrap();
    }
    let url = invite.to_url("https://tonk.example/connect").unwrap();
    let unsafe_url = url.replace("https://tonk.example", "http://evil.example");
    let error = AgentInvite::parse_url(&unsafe_url, &scopes, &remote(), now())
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(!message.contains("tonk-agent-v1="));
    assert!(!message.contains("evil.example"));
}

#[dialog_common::test]
async fn connection_refuses_export_of_root_or_browser_ancestor_keys() {
    for seed in [[21; 32], [22; 32]] {
        let (chains, scopes) = fixture(seed).await;
        let error = AgentInvite::new(seed, chains, &scopes, &remote(), now())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("recipient_not_fresh"), "{error}");
    }
}
