//! Integration test for the native `AccountServer`: drives the full
//! happy path over real HTTP with `reqwest`, exercising the same route
//! surface, JSON shapes, and status codes as the Cloudflare Worker.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::Principal;
use tonk_account::handoff::{ConsumedLink, LinkCreateRequest, LinkSecretRequest, ResolvedLink};
use tonk_account_service::helpers::AccountServer;

const ROOT_PRF: [u8; 32] = [7u8; 32];
const DEVICE_SEED: [u8; 32] = [8u8; 32];

/// Build a device-signed invocation container for the account's first
/// device, using the production builder against the `root → device`
/// delegation minted for account creation.
async fn container(command: Vec<String>, args: BTreeMap<String, Promised>) -> Vec<u8> {
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let link = tonk_identity::delegation::mint_device_delegation(root, &device.did())
        .await
        .unwrap();
    tonk_identity::request::build_device_invocation(device, &link, command, args)
        .await
        .unwrap()
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
    let body = container(
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
    assert_eq!(devices[0]["did"], device_did);
    assert_eq!(devices[0]["name"], "laptop");
    assert_eq!(devices[0]["status"], "active");
    assert_eq!(devices[1]["did"], second_did);
    assert_eq!(devices[1]["name"], "phone");

    // The worker and CLI parse exactly these keys; renaming one is a
    // breaking wire change.
    for key in [
        "did",
        "name",
        "status",
        "delegationCid",
        "delegationHex",
        "createdAt",
    ] {
        assert!(
            devices[0].get(key).is_some(),
            "device list row is missing `{key}`"
        );
    }
    assert!(devices[0].get("created_at").is_none());
    assert!(devices[0].get("delegation_cid").is_none());
    assert_eq!(devices[1]["delegationHex"], ceremony.delegation_hex);

    // POST /devices/revoke -> the first device cuts off the second,
    // carrying a root-signed revocation of the second device's grant.
    // Cross-device revocation needs root attestation; a device-signed
    // artifact only ever names its own grant.
    let second_grant_cid = devices[1]["delegationCid"].as_str().unwrap().to_string();
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
    let body = container(
        vec!["account".into(), "device".into(), "revoke".into()],
        [
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

    let body = container(
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
    assert_eq!(devices[0]["status"], "active");
    assert_eq!(devices[1]["status"], "revoked");

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
    let ceremony =
        tonk_identity::ceremony::complete_link(root, token_hash, cli.did(), "terminal".into())
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
    let response = client
        .post(format!("{base}/links/consume"))
        .json(&LinkSecretRequest {
            secret: secret.to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    // POST /chains/put then POST /chains/get -> round-trip chain bytes.
    let chain_bytes = b"a delegation chain, backed up".to_vec();
    let mut put_args = BTreeMap::new();
    put_args.insert(
        "chain".to_string(),
        Promised::String(hex::encode(&chain_bytes)),
    );
    let body = container(
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
    let body = container(
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
    let keys: Vec<String> = response.json().await.unwrap();
    assert!(keys.contains(&key));

    let mut get_args = BTreeMap::new();
    get_args.insert("key".to_string(), Promised::String(key));
    let body = container(
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
}
