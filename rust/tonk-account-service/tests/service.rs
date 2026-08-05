//! Integration test for the native `AccountServer`: drives the full
//! happy path over real HTTP with `reqwest`, exercising the same route
//! surface, JSON shapes, and status codes as the Cloudflare Worker.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::{Did, Principal};
use tonk_account::backup::{
    ACCOUNT_SPOTS_CAPABILITY_HEADER, ACCOUNT_SPOTS_CAPABILITY_V1, AccountSpotBackup,
    AccountSpotSummary,
};
use tonk_account::handoff::{ConsumedLink, LinkCreateRequest, LinkSecretRequest, ResolvedLink};
use tonk_account_service::helpers::AccountServer;

const ROOT_PRF: [u8; 32] = [7u8; 32];
const DEVICE_SEED: [u8; 32] = [8u8; 32];

/// Build a device-signed invocation container for the account's first
/// device, using the production builder against the `root → device`
/// delegation minted for account creation.
async fn spot_backup(root: &Did, name: Option<&str>, remote: &str) -> Vec<u8> {
    let space = Ed25519Signer::import(&[42; 32]).await.unwrap();
    let subject = space.did();
    let delegation = DelegationBuilder::new()
        .issuer(space)
        .audience(root)
        .subject(Subject::Specific(subject))
        .command(vec![])
        .try_build()
        .await
        .unwrap();
    let chain = DelegationChain::new(delegation);
    serde_json::to_vec(&AccountSpotBackup {
        chain_hex: hex::encode(chain.to_bytes().unwrap()),
        remote_url: Some(remote.to_string()),
        revocation_url: None,
        name: name.map(str::to_string),
    })
    .unwrap()
}

async fn container_for(
    root_prf: [u8; 32],
    device_seed: [u8; 32],
    command: Vec<String>,
    args: BTreeMap<String, Promised>,
) -> Vec<u8> {
    let root = tonk_identity::derive::derive_root_signer(&root_prf)
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
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
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
        .issuer(device)
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

async fn account_creation(email: &str, code: &str) -> Vec<u8> {
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
        .await
        .unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        email.to_string(),
        code.to_string(),
        "credential".to_string(),
        device.did(),
        "laptop".to_string(),
        hex::encode(grant.to_bytes().unwrap()),
        "http://127.0.0.1:8080/ucan/".to_string(),
        None,
    )
    .await
    .unwrap();
    hex::decode(ceremony.invocation_hex).unwrap()
}

#[dialog_common::test]
async fn it_exposes_captured_codes_over_http() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let endpoint = format!("{}/_test/emails", server.endpoint);

    let response = client
        .post(format!("{}/codes", server.endpoint))
        .json(&serde_json::json!({ "email": "person@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let first: serde_json::Value = client
        .get(&endpoint)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first[0]["address"], "person@example.com");
    assert!(
        first[0]["code"]
            .as_str()
            .is_some_and(|code| code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
    );

    let second: serde_json::Value = client
        .get(endpoint)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second, first, "reading the inbox must not drain it");

    server.stop().await;
}

#[dialog_common::test]
async fn it_enforces_the_resend_cooldown_over_http() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let endpoint = format!("{}/codes", server.endpoint);

    let first = client
        .post(&endpoint)
        .json(&serde_json::json!({ "email": "person@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let second = client
        .post(endpoint)
        .json(&serde_json::json!({ "email": "person@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 429);
    let error: serde_json::Value = second.json().await.unwrap();
    assert_eq!(error["error"]["code"], "RATE_LIMITED");
    assert_eq!(error["error"]["message"], "rate limited");

    server.stop().await;
}

#[dialog_common::test]
async fn it_exhausts_verification_attempts_over_http() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let email = "person@example.com";
    client
        .post(format!("{}/codes", server.endpoint))
        .json(&serde_json::json!({ "email": email }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let correct = server.emails.0.lock().unwrap()[0].1.clone();
    let wrong = if correct == "000000" {
        "111111"
    } else {
        "000000"
    };

    for _ in 0..tonk_account_service::core::codes::MAX_ATTEMPTS {
        let response = client
            .post(format!("{}/accounts", server.endpoint))
            .header("Content-Type", "application/cbor")
            .body(account_creation(email, wrong).await)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        let error: serde_json::Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "UNAUTHORIZED");
        assert_eq!(error["error"]["message"], "invalid or expired code");
    }

    let response = client
        .post(format!("{}/accounts", server.endpoint))
        .header("Content-Type", "application/cbor")
        .body(account_creation(email, &correct).await)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let error: serde_json::Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "UNAUTHORIZED");
    assert_eq!(error["error"]["message"], "invalid or expired code");

    server.stop().await;
}

#[dialog_common::test]
async fn it_checks_email_availability_only_after_a_valid_code() {
    let server = AccountServer::start().await;
    let client = reqwest::Client::new();
    let existing = "existing@example.com";

    client
        .post(format!("{}/codes", server.endpoint))
        .json(&serde_json::json!({ "email": existing }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let first_code = server.emails.0.lock().unwrap().last().unwrap().1.clone();
    let created = client
        .post(format!("{}/accounts", server.endpoint))
        .header("Content-Type", "application/cbor")
        .body(account_creation(existing, &first_code).await)
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);

    client
        .post(format!("{}/codes", server.endpoint))
        .json(&serde_json::json!({ "email": existing }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let existing_code = server.emails.0.lock().unwrap().last().unwrap().1.clone();
    let conflict = client
        .post(format!("{}/accounts/preflight", server.endpoint))
        .json(&serde_json::json!({
            "email": existing,
            "code": existing_code,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);
    let error: serde_json::Value = conflict.json().await.unwrap();
    assert_eq!(error["error"]["code"], "CONFLICT");
    assert_eq!(
        error["error"]["message"],
        tonk_account_service::core::accounts::EMAIL_TAKEN
    );

    let available = "available@example.com";
    client
        .post(format!("{}/codes", server.endpoint))
        .json(&serde_json::json!({ "email": available }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let available_code = server.emails.0.lock().unwrap().last().unwrap().1.clone();
    for _ in 0..2 {
        let response = client
            .post(format!("{}/accounts/preflight", server.endpoint))
            .json(&serde_json::json!({
                "email": available,
                "code": available_code,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "a successful preflight must not consume the code"
        );
    }

    let wrong_code = if available_code == "000000" {
        "111111"
    } else {
        "000000"
    };
    let wrong = client
        .post(format!("{}/accounts/preflight", server.endpoint))
        .json(&serde_json::json!({
            "email": available,
            "code": wrong_code,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

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
    assert_eq!(error["error"]["message"], "invocation has expired");

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
    let response = reqwest::Client::new()
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/codes", server.endpoint),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let headers = response.headers();
    assert_eq!(headers["access-control-allow-origin"], "*");
    assert_eq!(headers["access-control-allow-methods"], "POST, OPTIONS");
    assert_eq!(headers["access-control-allow-headers"], "Content-Type");
    assert_eq!(
        headers["access-control-expose-headers"],
        "Content-Type, X-Tonk-Account-Spots"
    );

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

    let latest_code = || {
        let sent = server.emails.0.lock().unwrap();
        sent.iter()
            .rfind(|(to, _)| to == email)
            .map(|(_, code): &(String, String)| code.clone())
            .expect("a code was sent")
    };
    let request_code = async |client: &reqwest::Client| {
        client
            .post(format!("{base}/codes"))
            .json(&serde_json::json!({ "email": email }))
            .send()
            .await
            .unwrap()
    };
    let create = async |client: &reqwest::Client, prf: [u8; 32], seed: [u8; 32], code: String| {
        let root = tonk_identity::derive::derive_root_signer(&prf)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&seed).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let ceremony = tonk_identity::ceremony::create_account(
            root,
            email.into(),
            code,
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

    assert_eq!(request_code(&client).await.status(), 200);
    let response = create(&client, ROOT_PRF, DEVICE_SEED, latest_code()).await;
    assert_eq!(response.status(), 201);

    // A different passkey claiming the same address: the email is taken,
    // and the root DID is not, so the conflict is about the email.
    assert_eq!(request_code(&client).await.status(), 200);
    let response = create(&client, [10u8; 32], [12u8; 32], latest_code()).await;
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

    // POST /codes -> request a verification code, then read it back out
    // of the captured emails instead of receiving mail.
    let response = client
        .post(format!("{base}/codes"))
        .json(&serde_json::json!({ "email": "person@example.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let code = {
        let sent = server.emails.0.lock().unwrap();
        sent.iter()
            .find(|(email, _)| email == "person@example.com")
            .map(|(_, code): &(String, String)| code.clone())
            .expect("a code was sent to person@example.com")
    };

    // POST /accounts -> create the account from a root-signed ceremony.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
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
        code,
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
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let establishment = tonk_identity::ceremony::establish_account_repository(
        root,
        "https://other.example/ucan/".into(),
    )
    .await
    .unwrap();
    assert_ne!(establishment.descriptor_hex, expected_descriptor);
    let response = client
        .post(format!("{base}/account/repository/establish"))
        .body(hex::decode(establishment.invocation_hex).unwrap())
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
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
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
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let linked: serde_json::Value = response.json().await.unwrap();
    assert_eq!(linked["descriptorHex"], expected_descriptor);

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
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
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

    // The unauthenticated global endpoint verifies the same artifact and
    // treats an identical publication as idempotent.
    let response = client
        .post(format!("{base}/revocations"))
        .header("Content-Type", "application/cbor")
        .body(revocation.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let published: serde_json::Value = response.json().await.unwrap();
    assert_eq!(published["targetCid"], second_grant_cid);
    assert_eq!(published["artifactCid"], revoked["artifactCid"]);
    assert_eq!(published["stored"], false);

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

    // A native profile creates a bearer-secret handoff. The browser
    // resolves its metadata, completes it with the passkey root, and
    // the native caller consumes the resulting delegation exactly once.
    let secret = "1111111111111111111111111111111111111111111111111111111111111111";
    let token_hash = blake3::hash(&hex::decode(secret).unwrap())
        .to_hex()
        .to_string();
    let cli = Ed25519Signer::import(&[10u8; 32]).await.unwrap();
    let cli_did = cli.did().to_string();
    let response = client
        .post(format!("{base}/links"))
        .json(&LinkCreateRequest {
            token_hash: token_hash.clone(),
            device_did: cli_did.clone(),
            device_name: "terminal".to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let response = client
        .post(format!("{base}/links/resolve"))
        .json(&LinkSecretRequest {
            secret: secret.to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let pending: ResolvedLink = response.json().await.unwrap();
    assert_eq!(pending.token_hash, token_hash);
    assert_eq!(pending.device_did, cli_did);
    assert_eq!(pending.device_name, "terminal");

    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let ceremony = tonk_identity::ceremony::complete_link(
        root,
        token_hash.clone(),
        cli.did(),
        "terminal".into(),
    )
    .await
    .unwrap();
    let expected_delegation = ceremony.delegation_hex.clone();
    let response = client
        .post(format!("{base}/links/complete"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = client
        .post(format!("{base}/links/consume"))
        .json(&LinkSecretRequest {
            secret: secret.to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let consumed: ConsumedLink = response.json().await.unwrap();
    assert_eq!(consumed.delegation_hex, expected_delegation);
    assert_eq!(consumed.credential_id, "cred-1");
    assert_eq!(consumed.descriptor_hex, expected_descriptor);
    assert_eq!(consumed.attachment_id.len(), 64);
    let response = client
        .post(format!("{base}/links/consume"))
        .json(&LinkSecretRequest {
            secret: secret.to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<ConsumedLink>().await.unwrap(), consumed);

    let completed_link =
        DelegationChain::try_from(hex::decode(&consumed.delegation_hex).unwrap().as_slice())
            .unwrap();
    let activation = container_with_link(
        &cli,
        &completed_link,
        vec!["account".into(), "link".into(), "activate".into()],
        [
            (
                "tokenHash".to_string(),
                Promised::String(token_hash.clone()),
            ),
            (
                "attachmentId".to_string(),
                Promised::String(consumed.attachment_id.clone()),
            ),
        ]
        .into_iter()
        .collect(),
    )
    .await;
    let response = client
        .post(format!("{base}/links/activate"))
        .body(activation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = client
        .post(format!("{base}/links/consume"))
        .json(&LinkSecretRequest {
            secret: secret.to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    // Logout detaches the exact generation without presenting the reusable
    // account grant, and replay is idempotent.
    let root_did = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap()
        .did();
    let detach = tonk_account::detach::SignedDetachIntent::sign(
        &dialog_credentials::SignerCredential::from(cli.clone()),
        &root_did,
        &consumed.attachment_id,
        &completed_link.proof_cids()[0].to_string(),
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

    // POST /chains/put then POST /chains/get -> round-trip chain bytes.
    let chain_bytes = b"a delegation chain, backed up".to_vec();
    let mut put_args = BTreeMap::new();
    put_args.insert(
        "chain".to_string(),
        Promised::String(hex::encode(&chain_bytes)),
    );
    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "chain".into(), "put".into()],
        put_args,
    )
    .await;
    let response = client
        .post(format!("{base}/chains/put"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let put_result: serde_json::Value = response.json().await.unwrap();
    let key = put_result["key"].as_str().unwrap().to_string();

    // POST /chains/list -> the key we just put shows up.
    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "chain".into(), "list".into()],
        BTreeMap::new(),
    )
    .await;
    let response = client
        .post(format!("{base}/chains/list"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get(ACCOUNT_SPOTS_CAPABILITY_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(ACCOUNT_SPOTS_CAPABILITY_V1)
    );
    assert!(
        response
            .headers()
            .get("access-control-expose-headers")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(',').any(|name| name
                .trim()
                .eq_ignore_ascii_case(ACCOUNT_SPOTS_CAPABILITY_HEADER))),
        "CORS must expose the account-spots capability header"
    );
    let keys: Vec<String> = response.json().await.unwrap();
    assert!(keys.contains(&key));

    let mut get_args = BTreeMap::new();
    get_args.insert("key".to_string(), Promised::String(key));
    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "chain".into(), "get".into()],
        get_args,
    )
    .await;
    let response = client
        .post(format!("{base}/chains/get"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );
    let round_tripped = response.bytes().await.unwrap();
    assert_eq!(round_tripped.as_ref(), chain_bytes.as_slice());

    // Semantic account-spot inventory advances one subject head while the
    // generic list/get routes remain unchanged.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let named = spot_backup(&root.did(), Some("garden"), "https://one.example/ucan/").await;
    let mut args = BTreeMap::new();
    args.insert("chain".to_string(), Promised::String(hex::encode(&named)));
    let response = client
        .post(format!("{base}/chains/put"))
        .body(
            container_with_link(
                &device,
                &first_grant,
                vec!["account".into(), "chain".into(), "put".into()],
                args,
            )
            .await,
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let spots = || async {
        let body = container_with_link(
            &device,
            &first_grant,
            vec!["account".into(), "chain".into(), "spots".into()],
            BTreeMap::new(),
        )
        .await;
        client
            .post(format!("{base}/chains/spots"))
            .body(body)
            .send()
            .await
            .unwrap()
    };
    let rows: Vec<AccountSpotSummary> = spots().await.json().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name.as_deref(), Some("garden"));

    let renamed = spot_backup(&root.did(), Some("orchard"), "https://two.example/ucan/").await;
    let mut args = BTreeMap::new();
    args.insert("chain".to_string(), Promised::String(hex::encode(&renamed)));
    client
        .post(format!("{base}/chains/put"))
        .body(
            container_with_link(
                &device,
                &first_grant,
                vec!["account".into(), "chain".into(), "put".into()],
                args,
            )
            .await,
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let rows: Vec<AccountSpotSummary> = spots().await.json().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name.as_deref(), Some("orchard"));
    let selected_key = rows[0].key.clone().unwrap();

    let unnamed = spot_backup(&root.did(), None, "https://three.example/ucan/").await;
    let mut args = BTreeMap::new();
    args.insert("chain".to_string(), Promised::String(hex::encode(&unnamed)));
    client
        .post(format!("{base}/chains/put"))
        .body(
            container_with_link(
                &device,
                &first_grant,
                vec!["account".into(), "chain".into(), "put".into()],
                args,
            )
            .await,
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let rows: Vec<AccountSpotSummary> = spots().await.json().await.unwrap();
    assert_eq!(rows[0].name.as_deref(), Some("orchard"));
    assert_eq!(rows[0].key.as_deref(), Some(selected_key.as_str()));

    // A separately registered account on the same service sees no rows from
    // the first account's root-DID namespace.
    let other_email = "other@example.com";
    client
        .post(format!("{base}/codes"))
        .json(&serde_json::json!({ "email": other_email }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let other_code = {
        let sent = server.emails.0.lock().unwrap();
        sent.iter()
            .rfind(|(email, _)| email == other_email)
            .map(|(_, code)| code.clone())
            .unwrap()
    };
    let other_prf = [21; 32];
    let other_seed = [22; 32];
    let other_root = tonk_identity::derive::derive_root_signer(&other_prf)
        .await
        .unwrap();
    let other_device = Ed25519Signer::import(&other_seed).await.unwrap();
    let other_grant =
        tonk_identity::delegation::mint_device_delegation(other_root.clone(), &other_device.did())
            .await
            .unwrap();
    let other_ceremony = tonk_identity::ceremony::create_account(
        other_root,
        other_email.to_string(),
        other_code,
        "other-credential".to_string(),
        other_device.did(),
        "other-device".to_string(),
        hex::encode(other_grant.to_bytes().unwrap()),
        "http://127.0.0.1:8080/ucan/".to_string(),
        None,
    )
    .await
    .unwrap();
    client
        .post(format!("{base}/accounts"))
        .body(hex::decode(other_ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let response = client
        .post(format!("{base}/chains/spots"))
        .body(
            container_with_link(
                &other_device,
                &other_grant,
                vec!["account".into(), "chain".into(), "spots".into()],
                BTreeMap::new(),
            )
            .await,
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response
            .json::<Vec<AccountSpotSummary>>()
            .await
            .unwrap()
            .is_empty()
    );

    let body = container_with_link(
        &device,
        &first_grant,
        vec!["account".into(), "chain".into(), "get".into()],
        [("key".to_string(), Promised::String(selected_key))]
            .into_iter()
            .collect(),
    )
    .await;
    let fetched = client
        .post(format!("{base}/chains/get"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), renamed.as_slice());

    let revoked_body = tonk_identity::request::build_device_invocation(
        second,
        &second_grant,
        vec!["account".into(), "chain".into(), "spots".into()],
        BTreeMap::new(),
    )
    .await
    .unwrap();
    let rejected = client
        .post(format!("{base}/chains/spots"))
        .body(revoked_body)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 403);
}
