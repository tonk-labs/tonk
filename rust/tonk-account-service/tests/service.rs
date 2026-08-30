//! Integration test for the native `AccountServer`: drives the full
//! happy path over real HTTP with `reqwest`, exercising the same route
//! surface, JSON shapes, and status codes as the Cloudflare Worker.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::Principal;
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

async fn account_registration(email: &str) -> (Vec<u8>, Ed25519Signer, DelegationChain) {
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
    (hex::decode(ceremony.invocation_hex).unwrap(), device, grant)
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
    let body =
        container_with_expiration(vec!["account".into(), "summary".into()], expiration).await;
    let response = reqwest::Client::new()
        .post(format!("{}/account/summary", server.endpoint))
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
    let body =
        container_with_expiration(vec!["account".into(), "summary".into()], expiration).await;
    let response = reqwest::Client::new()
        .post(format!("{}/account/summary", server.endpoint))
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
    let response = reqwest::Client::new()
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/accounts", server.endpoint),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let headers = response.headers();
    assert_eq!(headers["access-control-allow-origin"], "*");
    assert_eq!(headers["access-control-allow-methods"], "POST, OPTIONS");
    assert_eq!(headers["access-control-allow-headers"], "Content-Type");
    assert_eq!(headers["access-control-expose-headers"], "Content-Type");

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
    let first_grant =
        tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root.clone(),
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

    // POST /devices/link -> a second browser attaches to the same
    // account, and gets its own grant rather than the first one's.
    let second = Ed25519Signer::generate().await.unwrap();
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
}
