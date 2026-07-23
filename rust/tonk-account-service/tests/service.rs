//! Integration test for the native `AccountServer`: drives the full
//! happy path over real HTTP with `reqwest`, exercising the same route
//! surface, JSON shapes, and status codes as the Cloudflare Worker.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_varsig::Principal;
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
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        "person@example.com".into(),
        code,
        "cred-1".into(),
        device.did(),
        "laptop".into(),
    )
    .await
    .unwrap();

    let response = client
        .post(format!("{base}/accounts"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let created: serde_json::Value = response.json().await.unwrap();
    assert!(created["accountId"].is_i64());

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
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

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
        .json(&serde_json::json!({
            "tokenHash": token_hash,
            "deviceDid": cli_did,
            "deviceName": "terminal"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let response = client
        .post(format!("{base}/links/resolve"))
        .json(&serde_json::json!({ "secret": secret }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let pending: serde_json::Value = response.json().await.unwrap();
    assert_eq!(pending["deviceDid"], cli_did);
    assert_eq!(pending["deviceName"], "terminal");

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
        .json(&serde_json::json!({ "secret": secret }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let consumed: serde_json::Value = response.json().await.unwrap();
    assert_eq!(consumed["delegationHex"], expected_delegation);
    let response = client
        .post(format!("{base}/links/consume"))
        .json(&serde_json::json!({ "secret": secret }))
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
