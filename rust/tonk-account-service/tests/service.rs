//! Integration test for the native `AccountServer`: drives the full
//! happy path over real HTTP with `reqwest`, exercising the same route
//! surface, JSON shapes, and status codes as the Cloudflare Worker.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::{BTreeMap, HashMap};

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::Principal;
use tonk_account::creation::AccountCreationFingerprintInput;
use tonk_account_service::helpers::AccountServer;

const ROOT_PRF: [u8; 32] = [7u8; 32];
const DEVICE_SEED: [u8; 32] = [8u8; 32];

/// Build a device-signed invocation container for the account's first
/// device, using the production builder against the `root → device`
/// delegation minted for account creation.
async fn container_for(
    root_prf: [u8; 32],
    device_seed: [u8; 32],
    command: Vec<String>,
    args: BTreeMap<String, Promised>,
) -> Vec<u8> {
    let root = dialog_credentials::Ed25519Signer::import(&root_prf)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&device_seed).await.unwrap();
    let link = tonk_identity::delegation::mint_device_delegation(root, &device.did())
        .await
        .unwrap();
    tonk_identity::request::build_device_invocation(device, &link, command, args)
        .await
        .unwrap()
}

async fn container(command: Vec<String>, args: BTreeMap<String, Promised>) -> Vec<u8> {
    container_for(ROOT_PRF, DEVICE_SEED, command, args).await
}

async fn container_with_link(
    device: &Ed25519Signer,
    link: &DelegationChain,
    command: Vec<String>,
    args: BTreeMap<String, Promised>,
) -> Vec<u8> {
    tonk_identity::request::build_device_invocation(device.clone(), link, command, args)
        .await
        .unwrap()
}

async fn container_with_expiration(command: Vec<String>, expiration: Timestamp) -> Vec<u8> {
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let root_did = root.did();
    let chain = tonk_identity::delegation::mint_device_delegation(root, &device.did())
        .await
        .unwrap();
    let delegation = chain.proofs().last().unwrap().clone();
    let cid = delegation.to_cid();
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(device))
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(BTreeMap::new())
        .proofs(vec![cid])
        .expiration(expiration)
        .try_build()
        .await
        .unwrap();
    let proofs = [(cid, std::sync::Arc::new(delegation))]
        .into_iter()
        .collect();
    InvocationChain::new(invocation, proofs).to_bytes().unwrap()
}

async fn account_registration_with_fingerprint(
    email: &str,
) -> (Vec<u8>, Ed25519Signer, DelegationChain, String) {
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
        .await
        .unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        email.to_string(),
        "credential".to_string(),
        device.did(),
        "laptop".to_string(),
        hex::encode(grant.to_bytes().unwrap()),
        "http://127.0.0.1:8080/ucan/".to_string(),
        None,
    )
    .await
    .unwrap();
    let delegation = grant.to_bytes().unwrap();
    let delegation_cid = grant.proof_cids()[0].to_string();
    let descriptor = hex::decode(ceremony.descriptor_hex.as_ref().unwrap()).unwrap();
    let fingerprint = AccountCreationFingerprintInput {
        email,
        root_did: &ceremony.root_did,
        credential_id: "credential",
        passkey: None,
        descriptor: &descriptor,
        device_did: &ceremony.device_did,
        device_name: "laptop",
        delegation_cid: &delegation_cid,
        delegation: &delegation,
    }
    .fingerprint()
    .to_hex();
    (
        hex::decode(ceremony.invocation_hex).unwrap(),
        device,
        grant,
        fingerprint,
    )
}

async fn account_registration(email: &str) -> (Vec<u8>, Ed25519Signer, DelegationChain) {
    let (invocation, device, grant, _) = account_registration_with_fingerprint(email).await;
    (invocation, device, grant)
}

async fn account_creation(email: &str) -> Vec<u8> {
    account_registration(email).await.0
}

/// Deletion finalizes with the device's delegated authority — the
/// exact `root → device` grant registered at creation — not a
/// root-signed passkey ceremony.
async fn account_deletion(device: &Ed25519Signer, link: &DelegationChain, email: &str) -> Vec<u8> {
    container_with_link(
        device,
        link,
        vec!["account".into(), "delete".into()],
        BTreeMap::from([(
            "confirmedEmail".to_string(),
            Promised::String(email.to_string()),
        )]),
    )
    .await
}

async fn account_setup_status_invocation(
    device: &Ed25519Signer,
    link: &DelegationChain,
    fingerprint: &str,
) -> Vec<u8> {
    container_with_link(
        device,
        link,
        vec!["account".into(), "setup".into(), "status".into()],
        BTreeMap::from([(
            "createFingerprint".to_string(),
            Promised::String(fingerprint.to_string()),
        )]),
    )
    .await
}

#[dialog_common::test]
async fn it_recovers_a_lost_creation_response_and_reports_setup_status() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let (creation, device, link, expected_fingerprint) =
        account_registration_with_fingerprint("recover@example.com").await;

    // The provider commits, but the caller loses this response body.
    let first = client
        .post(format!("{}/accounts", server.endpoint))
        .body(creation.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 201);
    drop(first);

    // The caller retained the shared-contract fingerprint before sending the
    // request, so it can authenticate and recover the committed winner without
    // first needing any provider response.
    let status = account_setup_status_invocation(&device, &link, &expected_fingerprint).await;
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(status)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let accepted: serde_json::Value = response.json().await.unwrap();
    assert_eq!(accepted["status"], "accepted");
    assert!(accepted["accountId"].is_i64());
    assert!(accepted["descriptorHex"].is_string());
    assert_eq!(accepted["createFingerprint"], expected_fingerprint);

    let replayed = client
        .post(format!("{}/accounts", server.endpoint))
        .body(creation)
        .send()
        .await
        .unwrap();
    assert_eq!(replayed.status(), 200);
    let replayed: serde_json::Value = replayed.json().await.unwrap();
    assert_eq!(replayed["reused"], true);
    let fingerprint = replayed["createFingerprint"]
        .as_str()
        .expect("replay carries its creation fingerprint")
        .to_string();
    assert_eq!(fingerprint, expected_fingerprint);
    assert_eq!(accepted["accountId"], replayed["accountId"]);
    assert_eq!(accepted["descriptorHex"], replayed["descriptorHex"]);

    server.stop().await;
}

#[dialog_common::test]
async fn it_keeps_setup_status_private_and_structured() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();

    // A valid proof for a root with no provider row receives Absent. There is
    // no root or email argument with which to probe somebody else's account.
    let absent_root = dialog_credentials::Ed25519Signer::import(&[20u8; 32])
        .await
        .unwrap();
    let absent_device = Ed25519Signer::import(&[21u8; 32]).await.unwrap();
    let absent_link =
        tonk_identity::delegation::mint_device_delegation(absent_root, &absent_device.did())
            .await
            .unwrap();
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(account_setup_status_invocation(&absent_device, &absent_link, &"ab".repeat(32)).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let absent: serde_json::Value = response.json().await.unwrap();
    assert_eq!(absent, serde_json::json!({ "status": "absent" }));

    let (creation, device, link) = account_registration("private@example.com").await;
    let created = client
        .post(format!("{}/accounts", server.endpoint))
        .body(creation)
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created: serde_json::Value = created.json().await.unwrap();
    let fingerprint = created["createFingerprint"].as_str().unwrap();

    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(account_setup_status_invocation(&device, &link, &"cd".repeat(32)).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mismatch: serde_json::Value = response.json().await.unwrap();
    assert_eq!(mismatch, serde_json::json!({ "status": "mismatch" }));

    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(account_setup_status_invocation(&device, &link, &fingerprint.to_uppercase()).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let malformed: serde_json::Value = response.json().await.unwrap();
    assert_eq!(malformed["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        malformed["error"]["message"],
        "createFingerprint must be 32 bytes of lowercase hex"
    );

    // Another valid device proof under this root is indistinguishable from an
    // absent account; it cannot learn that a different first device exists.
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let other_device = Ed25519Signer::import(&[22u8; 32]).await.unwrap();
    let other_link =
        tonk_identity::delegation::mint_device_delegation(root.clone(), &other_device.did())
            .await
            .unwrap();
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(account_setup_status_invocation(&other_device, &other_link, fingerprint).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let wrong_device: serde_json::Value = response.json().await.unwrap();
    assert_eq!(wrong_device, absent);

    // Nor can the first device substitute a newly issued stable delegation
    // CID. The proof is canonical, but it is not the persisted first grant.
    let alternate = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root.clone()))
        .audience(&device.did())
        .subject(Subject::Any)
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    let alternate = DelegationChain::new(alternate);
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(account_setup_status_invocation(&device, &alternate, fingerprint).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let wrong_cid: serde_json::Value = response.json().await.unwrap();
    assert_eq!(wrong_cid, absent);

    // Deletion removes the account and first-device row, but the caller still
    // holds the original public proof. It receives the exact same wire result.
    let response = client
        .post(format!("{}/account/delete", server.endpoint))
        .body(account_deletion(&device, &link, "private@example.com").await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(account_setup_status_invocation(&device, &link, fingerprint).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let deleted: serde_json::Value = response.json().await.unwrap();
    assert_eq!(deleted, absent);

    // Direct root proof and a valid device invocation for another command are
    // both refused at the authentication boundary.
    let root_did = root.did();
    let direct = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&root_did)
        .subject(&root_did)
        .command(vec!["account".into(), "setup".into(), "status".into()])
        .arguments(BTreeMap::from([(
            "createFingerprint".to_string(),
            Promised::String(fingerprint.to_string()),
        )]))
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .unwrap();
    let direct = InvocationChain::new(direct, HashMap::new())
        .to_bytes()
        .unwrap();
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(direct)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    let wrong_command = container_with_link(
        &device,
        &link,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await;
    let response = client
        .post(format!("{}/accounts/setup-status", server.endpoint))
        .body(wrong_command)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let wrong_command: serde_json::Value = response.json().await.unwrap();

    for body in [malformed, wrong_command] {
        let rendered = body.to_string();
        for leak in [
            "UNIQUE constraint",
            "accounts.",
            "devices.",
            "SQLITE_",
            "D1Error",
            "private@example.com",
            "accountId",
            "descriptorHex",
        ] {
            assert!(
                !rendered.contains(leak),
                "setup refusal leaked {leak:?}: {rendered}"
            );
        }
    }

    server.stop().await;
}

#[dialog_common::test]
async fn it_deletes_account_state_and_releases_the_email_over_http() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let email = "delete-me@example.com";

    let (creation, device, link) = account_registration(email).await;
    client
        .post(format!("{}/accounts", server.endpoint))
        .body(creation)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let deleted = client
        .post(format!("{}/account/delete", server.endpoint))
        .body(account_deletion(&device, &link, email).await)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 200);
    let receipt: serde_json::Value = deleted.json().await.unwrap();
    assert_eq!(receipt["email"], email);

    let recreated = client
        .post(format!("{}/accounts", server.endpoint))
        .body(account_creation(email).await)
        .send()
        .await
        .unwrap();
    assert_eq!(recreated.status(), 201);

    server.stop().await;
}

#[dialog_common::test]
async fn it_rejects_a_mismatched_command() {
    let server = AccountServer::start().await;
    let body = container(
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{}/accounts", server.endpoint))
        .header("Content-Type", "application/cbor")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let error: serde_json::Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "FORBIDDEN");

    server.stop().await;
}

#[dialog_common::test]
async fn it_rejects_an_expired_invocation() {
    let server = AccountServer::start().await;
    let expiration =
        Timestamp::new(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
    let body = container_with_expiration(
        vec!["account".into(), "device".into(), "list".into()],
        expiration,
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{}/devices/list", server.endpoint))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let error: serde_json::Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "UNAUTHORIZED");
    // Expiry is dialog's finding now, judged against the verification
    // context's clock rather than a bound compared here, so the wording is
    // theirs. The refusal and its code are what this pins.
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.to_lowercase().contains("expired"),
        "expected an expiry refusal, got: {message}"
    );

    server.stop().await;
}

#[dialog_common::test]
async fn it_rejects_an_over_long_expiration_window() {
    let server = AccountServer::start().await;
    let expiration = Timestamp::new(SystemTime::now() + Duration::from_secs(10 * 60)).unwrap();
    let body = container_with_expiration(
        vec!["account".into(), "device".into(), "list".into()],
        expiration,
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{}/devices/list", server.endpoint))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let error: serde_json::Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "UNAUTHORIZED");
    assert_eq!(
        error["error"]["message"],
        "invocation expiration exceeds the five-minute ceremony window plus skew allowance"
    );

    server.stop().await;
}

#[dialog_common::test]
async fn it_answers_preflight_with_cors_headers() {
    let server = AccountServer::start().await;
    for path in ["/accounts", "/accounts/setup-status"] {
        let response = reqwest::Client::new()
            .request(
                reqwest::Method::OPTIONS,
                format!("{}{path}", server.endpoint),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 204, "wrong preflight status for {path}");
        let headers = response.headers();
        assert_eq!(headers["access-control-allow-origin"], "*");
        assert_eq!(headers["access-control-allow-methods"], "POST, OPTIONS");
        assert_eq!(headers["access-control-allow-headers"], "Content-Type");
        assert_eq!(headers["access-control-expose-headers"], "Content-Type");
    }

    server.stop().await;
}

/// Creating a second account for an already-registered email address
/// returns 409 with a message meant for the person reading it, and none
/// of the database's own constraint text.
#[dialog_common::test]
async fn it_explains_an_already_registered_email_over_http() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let base = server.endpoint.clone();
    let email = "person@example.com";

    let create = async |client: &reqwest::Client, prf: [u8; 32], seed: [u8; 32]| {
        let root = dialog_credentials::Ed25519Signer::import(&prf)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&seed).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let ceremony = tonk_identity::ceremony::create_account(
            root,
            email.into(),
            "cred".into(),
            device.did(),
            "laptop".into(),
            hex::encode(grant.to_bytes().unwrap()),
            "http://127.0.0.1:8080/ucan/".into(),
            None,
        )
        .await
        .unwrap();
        client
            .post(format!("{base}/accounts"))
            .body(hex::decode(ceremony.invocation_hex).unwrap())
            .send()
            .await
            .unwrap()
    };

    let response = create(&client, ROOT_PRF, DEVICE_SEED).await;
    assert_eq!(response.status(), 201);

    // A different passkey claiming the same address: the email is taken,
    // and the root DID is not, so the conflict is about the email.
    let response = create(&client, [10u8; 32], [12u8; 32]).await;
    assert_eq!(response.status(), 409);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "CONFLICT");
    assert_eq!(
        body["error"]["message"],
        tonk_account_service::core::accounts::EMAIL_TAKEN
    );
    let rendered = body.to_string();
    for leak in ["UNIQUE constraint", "accounts.", "SQLITE_", "D1Error"] {
        assert!(
            !rendered.contains(leak),
            "response leaked {leak:?}: {rendered}"
        );
    }
}

#[dialog_common::test]
async fn it_drives_the_full_ceremony_over_http() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let base = server.endpoint.clone();

    // POST /accounts -> create the account from a root-signed ceremony.
    // No code ceremony: address control is proven by customer activation
    // at the access service.
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let device_did = device.did().to_string();
    let first_grant =
        tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        "person@example.com".into(),
        "cred-1".into(),
        device.did(),
        "laptop".into(),
        hex::encode(first_grant.to_bytes().unwrap()),
        "http://127.0.0.1:8080/ucan/".into(),
        Some(tonk_identity::ceremony::PasskeyCreationMetadata {
            created_at: 1_754_380_800,
            created_on: "Chrome on macOS".into(),
        }),
    )
    .await
    .unwrap();

    let expected_descriptor = ceremony.descriptor_hex.clone().unwrap();
    let response = client
        .post(format!("{base}/accounts"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let created: serde_json::Value = response.json().await.unwrap();
    assert!(created["accountId"].is_i64());
    assert_eq!(created["descriptorHex"], expected_descriptor);

    // The account summary reveals verified account facts only to an active
    // device. Passkey facts are the values witnessed at credential creation,
    // not inferred from this account or device registration time.
    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "summary".into()],
        BTreeMap::new(),
    )
    .await;
    let response = client
        .post(format!("{base}/account/summary"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let summary: serde_json::Value = response.json().await.unwrap();
    assert_eq!(summary["email"], "person@example.com");
    assert_eq!(summary["passkey"]["createdAt"], 1_754_380_800_u64);
    assert_eq!(summary["passkey"]["createdOn"], "Chrome on macOS");

    // Establishment is set-if-absent: a later valid candidate receives
    // the stored creation winner, never its own bytes.
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    // Built inline: the browser no longer carries an establish ceremony
    // (legacy pre-descriptor accounts reset), but the server keeps the
    // set-if-absent arbitration, so the test speaks the wire form.
    let descriptor =
        tonk_account::AccountRepositoryDescriptorV1::sign(&root, "https://other.example/ucan/")
            .await
            .unwrap();
    let candidate_hex = hex::encode(descriptor.bytes());
    assert_ne!(candidate_hex, expected_descriptor);
    let root_did = root.did();
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&root_did)
        .subject(&root_did)
        .command(vec![
            "account".into(),
            "repository".into(),
            "establish".into(),
        ])
        .arguments(BTreeMap::from([(
            "repositoryDescriptor".to_string(),
            Promised::String(candidate_hex),
        )]))
        .proofs(vec![])
        .issue_now()
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .unwrap();
    let body = InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .unwrap();
    let response = client
        .post(format!("{base}/account/repository/establish"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let established: serde_json::Value = response.json().await.unwrap();
    assert_eq!(established["descriptorHex"], expected_descriptor);
    assert_eq!(established["created"], false);

    // A fresh browser profile can self-link directly from the root
    // ceremony; it does not need an already registered device to sign.
    let second = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
    let second_did = second.did().to_string();
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let ceremony = tonk_identity::ceremony::link_device(root, second.did(), "phone".into())
        .await
        .unwrap();
    let second_grant_bytes = hex::decode(&ceremony.delegation_hex).unwrap();
    let second_grant =
        dialog_ucan_core::DelegationChain::try_from(second_grant_bytes.as_slice()).unwrap();
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(&ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let linked: serde_json::Value = response.json().await.unwrap();
    assert_eq!(linked["descriptorHex"], expected_descriptor);
    let second_attachment = linked["attachmentId"].as_str().unwrap();

    // Browser sign-out leaves the account-service attachment active. A later
    // login with the same passkey mints a new invocation and nonce-bearing
    // root-to-device grant, but must recover the active generation.
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let relink = tonk_identity::ceremony::link_device(root, second.did(), "phone".into())
        .await
        .unwrap();
    assert_ne!(relink.delegation_hex, ceremony.delegation_hex);
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(relink.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let relinked: serde_json::Value = response.json().await.unwrap();
    assert_eq!(relinked["attachmentId"], second_attachment);

    // POST /devices/list -> the newly registered device shows up.
    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await;
    let response = client
        .post(format!("{base}/devices/list"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let devices: serde_json::Value = response.json().await.unwrap();
    let devices = devices.as_array().unwrap();
    assert_eq!(devices.len(), 2);
    let first_row = devices.iter().find(|row| row["did"] == device_did).unwrap();
    let second_row = devices.iter().find(|row| row["did"] == second_did).unwrap();
    assert_eq!(first_row["name"], "laptop");
    assert_eq!(first_row["status"], "active");
    assert_eq!(second_row["name"], "phone");

    // The worker and CLI parse exactly these keys; renaming one is a
    // breaking wire change.
    for key in [
        "attachmentId",
        "did",
        "name",
        "status",
        "delegationCid",
        "delegationHex",
        "createdAt",
    ] {
        assert!(
            first_row.get(key).is_some(),
            "device list row is missing `{key}`"
        );
    }
    assert!(first_row.get("created_at").is_none());
    assert!(first_row.get("delegation_cid").is_none());
    assert_eq!(second_row["delegationHex"], ceremony.delegation_hex);

    // POST /devices/revoke -> the first device cuts off the second,
    // carrying a root-signed revocation of the second device's grant.
    // Cross-device revocation needs root attestation; a device-signed
    // artifact only ever names its own grant.
    let second_grant_cid = second_row["delegationCid"].as_str().unwrap().to_string();
    assert_eq!(second_grant_cid, second_grant.proof_cids()[0].to_string());
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let revocation = tonk_identity::revocation::mint_root_revocation(
        root,
        &second_grant,
        &second_grant.proof_cids()[0],
    )
    .await
    .unwrap();
    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "device".into(), "revoke".into()],
        [
            (
                "attachmentId".to_owned(),
                Promised::String(second_row["attachmentId"].as_str().unwrap().to_string()),
            ),
            ("did".to_owned(), Promised::String(second_did.clone())),
            (
                "revocation".to_owned(),
                Promised::String(hex::encode(&revocation)),
            ),
        ]
        .into_iter()
        .collect(),
    )
    .await;
    let response = client
        .post(format!("{base}/devices/revoke"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let revoked: serde_json::Value = response.json().await.unwrap();
    assert_eq!(revoked["attestation"], "root");
    assert_eq!(revoked["projection"], "updated");
    assert_eq!(revoked["targetDid"], second_did);
    assert_eq!(revoked["targetCid"], second_grant_cid);
    assert_eq!(revoked["published"], true);

    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await;
    let response = client
        .post(format!("{base}/devices/list"))
        .body(body)
        .send()
        .await
        .unwrap();
    let devices: serde_json::Value = response.json().await.unwrap();
    let devices = devices.as_array().unwrap();
    assert_eq!(
        devices.iter().find(|row| row["did"] == device_did).unwrap()["status"],
        "active"
    );
    assert_eq!(
        devices.iter().find(|row| row["did"] == second_did).unwrap()["status"],
        "revoked"
    );

    // A command-line profile is registered through the same root ceremony
    // the browser runs when it approves a `tonk account login` callback.
    let cli = Ed25519Signer::import(&[10u8; 32]).await.unwrap();
    let root = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap();
    let ceremony = tonk_identity::ceremony::link_device(root, cli.did(), "terminal".into())
        .await
        .unwrap();
    let cli_grant_bytes = hex::decode(&ceremony.delegation_hex).unwrap();
    let cli_grant = DelegationChain::try_from(cli_grant_bytes.as_slice()).unwrap();
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let linked: serde_json::Value = response.json().await.unwrap();
    assert_eq!(linked["descriptorHex"], expected_descriptor);
    let cli_attachment = linked["attachmentId"].as_str().unwrap().to_string();

    // Logout detaches the exact generation without presenting the reusable
    // account grant, and replay is idempotent.
    let root_did = dialog_credentials::Ed25519Signer::import(&ROOT_PRF)
        .await
        .unwrap()
        .did();
    let detach = tonk_account::detach::SignedDetachIntent::sign(
        &dialog_credentials::SignerCredential::from(cli.clone()),
        &root_did,
        &cli_attachment,
        &cli_grant.proof_cids()[0].to_string(),
        1,
    )
    .await
    .unwrap();
    let response = client
        .post(format!("{base}/devices/detach"))
        .json(&detach)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["outcome"],
        "detached"
    );
    let response = client
        .post(format!("{base}/devices/detach"))
        .json(&detach)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["outcome"],
        "alreadyDetached"
    );
}
