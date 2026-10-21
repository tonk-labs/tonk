//! Regression spike: real email lookup, worker mint/join, and reshare
//! provenance against the local access service. No production discovery API.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use dialog_capability::Subject;
use dialog_credentials::Ed25519Signer;
use dialog_effects::Use;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use dialog_varsig::{Did, Principal};
use serde_json::{Value, json};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_invite::{Invite, InviteAudience};
use tonk_schema::{Invitation, MemberRole, Membership};
use tower::ServiceExt;

use super::{AppState, CreateInviteResponse, RepositoryInfo, api_router_with_state};

async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .extension(crate::axum::RequestOrigin::parse("http://localhost/").unwrap())
                .header("content-type", "application/json")
                .header("origin", "http://localhost")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    // Do not print successful bodies: mint responses contain invitation links.
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!("{method} {path} returned {status} with a non-JSON body: {error}")
    });
    (status, body)
}

/// Test-only glue for the product seam that does not exist yet. A pending
/// registration is HTTP-successful too, so insist on confirmed status before
/// extracting the root. Do not turn missing/unconfirmed discovery into an open
/// invitation. The trusted service is explicit, not supplied by the document.
async fn confirmed_root(
    service: &AccessServiceAddress,
    email: &str,
) -> Result<Did, reqwest::StatusCode> {
    let origin = url::Url::parse(&service.access_service_url).unwrap();
    let expected = tonk_access_service::lookup::customer_did(origin.authority(), email).unwrap();
    let (domain, local) = super::email_status::split_address(email).unwrap();
    // Match the existing worker lookup: did:web resolution decodes the DID's
    // segments, so the HTTP path carries a literal + rather than DID encoding.
    let endpoint = format!(
        "{}/customer/{}/{}/did.json",
        service.access_service_url, domain, local,
    );
    let response = reqwest::get(endpoint).await.unwrap();
    if response.status() != reqwest::StatusCode::OK {
        return Err(response.status());
    }
    let document: Value = response.json().await.unwrap();
    assert_eq!(document["id"], expected);
    assert_eq!(document["status"], "Active");
    let root: Did = document["alsoKnownAs"][0]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        format!(
            "did:key:{}",
            document["verificationMethod"][0]["publicKeyMultibase"]
                .as_str()
                .unwrap()
        ),
        root.to_string(),
    );
    Ok(root)
}

async fn mint(app: &Router, space: &str, recipient: &Did) -> String {
    let (status, body) = request(
        app,
        "POST",
        &format!("/api/repository/{space}/invite"),
        json!({ "recipientRoot": recipient }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "scoped mint failed: {body}");
    let response: CreateInviteResponse = serde_json::from_value(body).unwrap();
    match response {
        CreateInviteResponse::Scoped {
            url,
            recipient_root,
        } => {
            assert_eq!(&recipient_root, recipient);
            assert!(
                url.fragment().is_none(),
                "no bearer seed in a targeted invite"
            );
            url.to_string()
        }
        CreateInviteResponse::Open { .. } => panic!("a targeted request became an open invitation"),
    }
}

async fn sync(app: &Router, space: &str, direction: &str) {
    let (status, body) = request(
        app,
        "POST",
        &format!("/api/repository/{space}/branch/main/sync/{direction}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{direction} failed: {body}");
}

async fn has_authority(state: &AppState, space: &Did) -> bool {
    let tonk = state.read().await;
    tonk.profile
        .access()
        .prove(Subject::from(space.clone()).attenuate(Use))
        .audience(&tonk.operator)
        .perform(&tonk.operator)
        .await
        .is_ok()
}

async fn profile_revision(state: &AppState) -> Option<dialog_artifacts::Revision> {
    let tonk = state.read().await;
    tonk.reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .unwrap()
        .handle()
        .revision()
}

#[dialog_common::test]
async fn email_lookup_to_targeted_join_and_reshare_attribution() {
    let (alice, service, alice_root, remote) =
        super::account_state::tests::ready_account_state(None).await;
    let bob_root = Ed25519Signer::generate().await.unwrap();
    let carol_root = Ed25519Signer::generate().await.unwrap();
    let mallory_root = Ed25519Signer::generate().await.unwrap();
    let email = "bob+discovery@example.com";

    assert_eq!(
        confirmed_root(&service.address, "missing@example.com").await,
        Err(reqwest::StatusCode::NOT_FOUND),
    );
    service
        .address
        .enroll_customer(&bob_root, email)
        .await
        .unwrap();
    assert_eq!(
        confirmed_root(&service.address, email).await,
        Err(reqwest::StatusCode::ACCEPTED),
        "an unconfirmed registration is not a verified recipient"
    );
    service.address.confirm_email(email).await.unwrap();
    let bob_did = confirmed_root(&service.address, email).await.unwrap();
    assert_eq!(bob_did, bob_root.did());
    let refusal = service
        .address
        .enroll_customer(&mallory_root, " BOB+DISCOVERY@EXAMPLE.COM ")
        .await
        .expect_err("another account cannot take Bob's confirmed address");
    assert!(refusal.to_string().contains("409"));
    assert!(refusal.to_string().contains("AddressTaken"));
    assert_eq!(
        confirmed_root(&service.address, email).await.unwrap(),
        bob_did,
    );
    eprintln!("discovery spike: confirmed email resolves; another account cannot take it");
    service
        .address
        .activate_customer(&carol_root, "carol@example.com")
        .await
        .unwrap();
    service
        .address
        .activate_customer(&mallory_root, "mallory@example.com")
        .await
        .unwrap();

    let (alice_app, _alice_state, _alice_hub) = api_router_with_state(alice);
    let (bob_app, bob_state, _bob_hub) = api_router_with_state(
        super::account_state::tests::device_on(&bob_root, &remote, None).await,
    );
    let (carol_app, carol_state, _carol_hub) = api_router_with_state(
        super::account_state::tests::device_on(&carol_root, &remote, None).await,
    );
    let (mallory_app, mallory_state, _mallory_hub) = api_router_with_state(
        super::account_state::tests::device_on(&mallory_root, &remote, None).await,
    );
    let (status, body) = request(
        &alice_app,
        "PUT",
        "/api/repository/discovery-spike",
        json!({ "branch": { "main": {} } }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "space creation failed: {body}");
    let info: RepositoryInfo = serde_json::from_value(body).unwrap();
    let space = info.name;
    let space_did = info.subject;
    // Provision only the test space through the fixture shortcut. The account
    // email enrollment and confirmation above use the actual service ceremony.
    service
        .address
        .provision_subject(space_did.as_str())
        .await
        .unwrap();
    let config = tonk_worker_api::RepositoryConfiguration::default()
        .remote(
            "origin",
            tonk_worker_api::RemoteConfiguration::ucan(&remote),
        )
        .branch(
            "main",
            tonk_worker_api::BranchConfiguration::default().upstream("origin", "main"),
        );
    let (status, body) = request(
        &alice_app,
        "POST",
        &format!("/api/repository/{space}/remote"),
        serde_json::to_value(config).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "attach remote failed: {body}");

    let invitation = mint(&alice_app, &space, &bob_did).await;
    sync(&alice_app, &space, "push").await;
    let parsed = Invite::parse_url(&invitation).await.unwrap();
    assert!(matches!(parsed.audience, InviteAudience::Scoped));
    assert_eq!(parsed.chain.audience(), &bob_did);
    assert_eq!(
        Invitation::from_chain(&parsed.chain)
            .unwrap()
            .inviter
            .0
            .to_string(),
        alice_root.did().to_string(),
    );

    let before = profile_revision(&mallory_state).await;
    assert!(!has_authority(&mallory_state, &space_did).await);
    let (status, body) = request(
        &mallory_app,
        "POST",
        "/api/profile/join",
        json!({ "url": invitation }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "wrong account response: {body}"
    );
    assert!(body.to_string().contains("different identity"));
    assert_eq!(profile_revision(&mallory_state).await, before);
    assert!(!has_authority(&mallory_state, &space_did).await);
    eprintln!("discovery spike: wrong account rejected without profile or authority changes");

    let (status, body) = request(
        &bob_app,
        "POST",
        "/api/profile/join",
        json!({ "url": invitation }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "intended recipient join failed: {body}"
    );
    assert!(has_authority(&bob_state, &space_did).await);
    eprintln!("discovery spike: intended recipient joined the seeded space");

    let carol_did = confirmed_root(&service.address, "carol@example.com")
        .await
        .unwrap();
    let reshared = mint(&bob_app, &space, &carol_did).await;
    let parsed = Invite::parse_url(&reshared).await.unwrap();
    assert_eq!(parsed.chain.audience(), &carol_did);
    // Characterize the actual minted chain, not a synthetic list of hops.
    assert!(
        parsed
            .chain
            .proofs()
            .any(|proof| proof.audience() == &bob_did)
    );
    let recorded_inviter = Invitation::from_chain(&parsed.chain).unwrap().inviter.0;
    assert_eq!(recorded_inviter.to_string(), bob_did.to_string());
    assert_ne!(recorded_inviter.to_string(), alice_root.did().to_string());
    sync(&bob_app, &space, "push").await;
    let (status, body) = request(
        &carol_app,
        "POST",
        "/api/profile/join",
        json!({ "url": reshared }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "reshared invitation join failed: {body}"
    );
    assert!(has_authority(&carol_state, &space_did).await);
    let (status, body) = request(
        &carol_app,
        "GET",
        &format!("/api/repository/{space}"),
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let info: RepositoryInfo = serde_json::from_value(body).unwrap();
    let member = info
        .members
        .iter()
        .find(|member| member.did == carol_did.to_string())
        .unwrap();
    assert_eq!(member.invited_by.as_deref(), Some(bob_did.as_str()));
    let bob_member = info
        .members
        .iter()
        .find(|member| member.did == bob_did.as_str())
        .unwrap();
    assert_eq!(
        bob_member.invited_by.as_deref(),
        Some(alice_root.did().as_str())
    );
    let alice_member = info
        .members
        .iter()
        .find(|member| member.did == alice_root.did().as_str())
        .unwrap();
    assert_eq!(alice_member.invited_by, None);
    // Read the same replicated content Carol joined: the sender fix must not
    // rewrite the separate original-owner role.
    {
        let tonk = carol_state.read().await;
        let repository = tonk
            .profile
            .space(&space)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let content = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let roles: Vec<MemberRole> = content
            .query()
            .select(Query::<MemberRole> {
                this: Term::var("this"),
                role: Term::var("role"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        let alice_membership = Membership::new(alice_root.did(), space_did.clone());
        let bob_membership = Membership::new(bob_did.clone(), space_did.clone());
        assert!(roles.contains(&MemberRole::founder(alice_membership.this.clone())));
        assert!(roles.contains(&MemberRole::member(bob_membership.this.clone())));
        assert!(!roles.contains(&MemberRole::founder(bob_membership.this)));
    }
    assert!(
        !info
            .members
            .iter()
            .any(|member| member.did == mallory_root.did().to_string())
    );
    eprintln!("discovery spike: Carol sees Bob as inviter; Alice remains founder");
    service.stop().await.unwrap();
}
