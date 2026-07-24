//! Integration test for the native `AccountServer`: drives the full
//! happy path over real HTTP with `reqwest`, exercising the same route
//! surface, JSON shapes, and status codes as the Cloudflare Worker.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{InvocationBuilder, InvocationChain};
use dialog_varsig::Principal;
use tonk_account_service::helpers::AccountServer;

const ROOT_PRF: [u8; 32] = [7u8; 32];
const NEW_ROOT_PRF: [u8; 32] = [9u8; 32];
const DEVICE_SEED: [u8; 32] = [8u8; 32];
const UNRELATED_NEW_ROOT_PRF: [u8; 32] = [11u8; 32];
const UNRELATED_OLD_ROOT_PRF: [u8; 32] = [13u8; 32];
const SURVIVING_DEVICE_SEED: [u8; 32] = [20u8; 32];

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

/// Build a root-signed ceremony container: issuer, audience, and subject
/// all equal `root`, with no proofs -- the same shape `authorize_root`
/// requires. Not yet available as a production builder (that lands with
/// the recovery ceremony's client-facing crate), so tests assemble it
/// directly.
async fn root_container(
    root: Ed25519Signer,
    command: Vec<String>,
    args: BTreeMap<String, Promised>,
) -> Vec<u8> {
    let root_did = root.did();
    let invocation = InvocationBuilder::new()
        .issuer(root)
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(args)
        .proofs(vec![])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .unwrap();
    InvocationChain::new(invocation, std::collections::HashMap::new())
        .to_bytes()
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

    // The worker and CLI parse exactly these keys; renaming one is a
    // breaking wire change.
    for key in ["did", "name", "status", "delegationCid", "createdAt"] {
        assert!(
            devices[0].get(key).is_some(),
            "device list row is missing `{key}`"
        );
    }
    assert!(devices[0].get("created_at").is_none());
    assert!(devices[0].get("delegation_cid").is_none());

    // POST /devices/revoke -> the first device cuts off the second.
    let body = container(
        vec!["account".into(), "device".into(), "revoke".into()],
        [("did".to_owned(), Promised::String(second_did.clone()))]
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

#[dialog_common::test]
async fn it_rotates_the_account_onto_a_new_root_over_http() {
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

    // POST /accounts/rotate -> flip the account onto a new root.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let rotation =
        tonk_identity::ceremony::rotate_account(old_root, new_root, "cred-2".into(), device.did())
            .await
            .unwrap();
    let response = client
        .post(format!("{base}/accounts/rotate"))
        .json(&serde_json::json!({
            "rotation": rotation.rotation_hex,
            "confirmation": rotation.confirmation_hex,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // A device-signed invocation under the NEW root succeeds.
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let new_link = tonk_identity::delegation::mint_device_delegation(new_root, &device.did())
        .await
        .unwrap();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &new_link,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await
    .unwrap();
    let response = client
        .post(format!("{base}/devices/list"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // The same invocation, signed against the OLD root, is rejected: the
    // account no longer resolves by its old root DID.
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
    assert_eq!(response.status(), 401);
}

#[dialog_common::test]
async fn it_rejects_a_rotation_whose_confirmation_names_a_different_root() {
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

    // Build a valid rotation container that names NEW_ROOT_PRF as the
    // new root, then pair it with the confirmation from a *different*
    // ceremony -- one that rotates the same old root onto an unrelated
    // third root. The rotation's `newRootDid` and the paired
    // confirmation's signer disagree, so the cross-check must reject it
    // even though both containers are independently valid, root-signed
    // rotate/confirm invocations.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let rotation =
        tonk_identity::ceremony::rotate_account(old_root, new_root, "cred-2".into(), device.did())
            .await
            .unwrap();

    let old_root_again = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let unrelated_new_root = tonk_identity::derive::derive_root_signer(&UNRELATED_NEW_ROOT_PRF)
        .await
        .unwrap();
    let unrelated = tonk_identity::ceremony::rotate_account(
        old_root_again,
        unrelated_new_root,
        "cred-3".into(),
        device.did(),
    )
    .await
    .unwrap();

    let response = client
        .post(format!("{base}/accounts/rotate"))
        .json(&serde_json::json!({
            "rotation": rotation.rotation_hex,
            "confirmation": unrelated.confirmation_hex,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // The account root did not change: a device-signed invocation under
    // the OLD root still succeeds.
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
}

#[dialog_common::test]
async fn it_recovers_the_account_via_surviving_device_and_new_root_over_http() {
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

    // POST /accounts -> create the account from a root-signed ceremony,
    // device A its first device.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_a = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let device_a_did = device_a.did().to_string();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        "person@example.com".into(),
        code,
        "cred-1".into(),
        device_a.did(),
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

    // Link device B (the surviving device) from the root ceremony.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_b = Ed25519Signer::import(&SURVIVING_DEVICE_SEED).await.unwrap();
    let device_b_did = device_b.did().to_string();
    let ceremony = tonk_identity::ceremony::link_device(root, device_b.did(), "phone".into())
        .await
        .unwrap();
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Capture device A's delegation CID before recovery, so we can prove
    // recovery leaves it untouched.
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
    let device_a_original_cid = devices
        .iter()
        .find(|d| d["did"] == device_a_did)
        .expect("device A is registered")["delegationCid"]
        .as_str()
        .unwrap()
        .to_string();

    // The old passkey is lost: recovery flips the account onto a freshly
    // created root, under device B's authority plus proof the new root
    // is controllable.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let old_root_did = old_root.did().to_string();
    let new_root_did = new_root.did().to_string();

    let device_link = tonk_identity::delegation::mint_device_delegation(old_root, &device_b.did())
        .await
        .unwrap();
    let fresh_delegation =
        tonk_identity::delegation::mint_device_delegation(new_root, &device_b.did())
            .await
            .unwrap();
    let device_delegation_hex = hex::encode(fresh_delegation.to_bytes().unwrap());

    let mut recovery_args = BTreeMap::new();
    recovery_args.insert(
        "newRootDid".to_string(),
        Promised::String(new_root_did.clone()),
    );
    recovery_args.insert(
        "newCredentialId".to_string(),
        Promised::String("cred-recovered".to_string()),
    );
    recovery_args.insert(
        "deviceDelegation".to_string(),
        Promised::String(device_delegation_hex.clone()),
    );
    let recovery_bytes = tonk_identity::request::build_device_invocation(
        device_b,
        &device_link,
        vec!["account".into(), "recover".into()],
        recovery_args,
    )
    .await
    .unwrap();

    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let mut confirmation_args = BTreeMap::new();
    confirmation_args.insert(
        "oldRootDid".to_string(),
        Promised::String(old_root_did.clone()),
    );
    let confirmation_bytes = root_container(
        new_root,
        vec!["account".into(), "recover".into(), "confirm".into()],
        confirmation_args,
    )
    .await;

    let response = client
        .post(format!("{base}/accounts/recover"))
        .json(&serde_json::json!({
            "recovery": hex::encode(recovery_bytes),
            "confirmation": hex::encode(confirmation_bytes),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Device B, signed under the new root, can now list devices.
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let device_b = Ed25519Signer::import(&SURVIVING_DEVICE_SEED).await.unwrap();
    let new_link = tonk_identity::delegation::mint_device_delegation(new_root, &device_b.did())
        .await
        .unwrap();
    let body = tonk_identity::request::build_device_invocation(
        device_b,
        &new_link,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await
    .unwrap();
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

    let device_a_row = devices
        .iter()
        .find(|d| d["did"] == device_a_did)
        .expect("device A is still registered");
    assert_eq!(device_a_row["status"], "active");
    assert_eq!(device_a_row["delegationCid"], device_a_original_cid);

    let device_b_row = devices
        .iter()
        .find(|d| d["did"] == device_b_did)
        .expect("device B is still registered");
    assert_eq!(device_b_row["status"], "active");

    // A device-signed call under the OLD root is rejected: the account
    // no longer resolves by its old root DID.
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
    assert_eq!(response.status(), 401);
}

#[dialog_common::test]
async fn it_rejects_a_rotation_confirmed_by_an_unrelated_ceremony() {
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

    // Build a valid rotation for the account (old root -> NEW_ROOT_PRF),
    // then pair it with the confirmation from an unrelated ceremony that
    // rotates a *different* old root onto that same new root. The
    // confirmation's signer still matches the rotation's declared
    // `newRootDid` (so the first arm of the cross-check is satisfied),
    // but its `oldRootDid` argument names the unrelated old root instead
    // of the account's actual one -- the other arm must catch it.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let rotation =
        tonk_identity::ceremony::rotate_account(old_root, new_root, "cred-2".into(), device.did())
            .await
            .unwrap();

    let unrelated_old_root = tonk_identity::derive::derive_root_signer(&UNRELATED_OLD_ROOT_PRF)
        .await
        .unwrap();
    let new_root_again = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let unrelated = tonk_identity::ceremony::rotate_account(
        unrelated_old_root,
        new_root_again,
        "cred-4".into(),
        device.did(),
    )
    .await
    .unwrap();

    let response = client
        .post(format!("{base}/accounts/rotate"))
        .json(&serde_json::json!({
            "rotation": rotation.rotation_hex,
            "confirmation": unrelated.confirmation_hex,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // The account root did not change: a device-signed invocation under
    // the OLD root still succeeds.
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
}

#[dialog_common::test]
async fn it_rejects_a_recovery_whose_confirmation_names_a_different_new_root() {
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

    // POST /accounts -> create the account from a root-signed ceremony,
    // device A its first device.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_a = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        "person@example.com".into(),
        code,
        "cred-1".into(),
        device_a.did(),
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

    // Link device B (the surviving device) from the root ceremony.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_b = Ed25519Signer::import(&SURVIVING_DEVICE_SEED).await.unwrap();
    let ceremony = tonk_identity::ceremony::link_device(root, device_b.did(), "phone".into())
        .await
        .unwrap();
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Build a valid device-signed recovery container that names
    // NEW_ROOT_PRF as the new root, then pair it with a confirmation
    // signed by an *unrelated* third root that correctly names the
    // account's old root. The recovery's `newRootDid` and the paired
    // confirmation's signer disagree, so the cross-check must reject it
    // even though both containers are independently valid, correctly
    // signed device/root invocations.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let old_root_did = old_root.did().to_string();
    let new_root_did = new_root.did().to_string();

    let device_link = tonk_identity::delegation::mint_device_delegation(old_root, &device_b.did())
        .await
        .unwrap();
    let fresh_delegation =
        tonk_identity::delegation::mint_device_delegation(new_root, &device_b.did())
            .await
            .unwrap();
    let device_delegation_hex = hex::encode(fresh_delegation.to_bytes().unwrap());

    let mut recovery_args = BTreeMap::new();
    recovery_args.insert(
        "newRootDid".to_string(),
        Promised::String(new_root_did.clone()),
    );
    recovery_args.insert(
        "newCredentialId".to_string(),
        Promised::String("cred-recovered".to_string()),
    );
    recovery_args.insert(
        "deviceDelegation".to_string(),
        Promised::String(device_delegation_hex.clone()),
    );
    let recovery_bytes = tonk_identity::request::build_device_invocation(
        device_b,
        &device_link,
        vec!["account".into(), "recover".into()],
        recovery_args,
    )
    .await
    .unwrap();

    // The confirmation is signed by an unrelated new root and correctly
    // names the account's actual old root -- only the `newRootDid` arm
    // of the cross-check is violated.
    let unrelated_new_root = tonk_identity::derive::derive_root_signer(&UNRELATED_NEW_ROOT_PRF)
        .await
        .unwrap();
    let mut confirmation_args = BTreeMap::new();
    confirmation_args.insert(
        "oldRootDid".to_string(),
        Promised::String(old_root_did.clone()),
    );
    let confirmation_bytes = root_container(
        unrelated_new_root,
        vec!["account".into(), "recover".into(), "confirm".into()],
        confirmation_args,
    )
    .await;

    let response = client
        .post(format!("{base}/accounts/recover"))
        .json(&serde_json::json!({
            "recovery": hex::encode(recovery_bytes),
            "confirmation": hex::encode(confirmation_bytes),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // The account root did not change: a device-signed invocation under
    // the OLD root still succeeds.
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
}

#[dialog_common::test]
async fn it_rejects_a_recovery_confirmed_for_a_different_account() {
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

    // POST /accounts -> create the account from a root-signed ceremony,
    // device A its first device.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_a = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        "person@example.com".into(),
        code,
        "cred-1".into(),
        device_a.did(),
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

    // Link device B (the surviving device) from the root ceremony.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_b = Ed25519Signer::import(&SURVIVING_DEVICE_SEED).await.unwrap();
    let ceremony = tonk_identity::ceremony::link_device(root, device_b.did(), "phone".into())
        .await
        .unwrap();
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Build a valid device-signed recovery container naming NEW_ROOT_PRF
    // as the new root, and pair it with a confirmation correctly signed
    // by that same new root -- satisfying the `newRootDid` arm -- but
    // whose `oldRootDid` argument names an unrelated account's old root
    // instead of this account's actual one. The other arm must catch it.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let new_root_did = new_root.did().to_string();

    let device_link = tonk_identity::delegation::mint_device_delegation(old_root, &device_b.did())
        .await
        .unwrap();
    let fresh_delegation =
        tonk_identity::delegation::mint_device_delegation(new_root, &device_b.did())
            .await
            .unwrap();
    let device_delegation_hex = hex::encode(fresh_delegation.to_bytes().unwrap());

    let mut recovery_args = BTreeMap::new();
    recovery_args.insert(
        "newRootDid".to_string(),
        Promised::String(new_root_did.clone()),
    );
    recovery_args.insert(
        "newCredentialId".to_string(),
        Promised::String("cred-recovered".to_string()),
    );
    recovery_args.insert(
        "deviceDelegation".to_string(),
        Promised::String(device_delegation_hex.clone()),
    );
    let recovery_bytes = tonk_identity::request::build_device_invocation(
        device_b,
        &device_link,
        vec!["account".into(), "recover".into()],
        recovery_args,
    )
    .await
    .unwrap();

    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let unrelated_old_root = tonk_identity::derive::derive_root_signer(&UNRELATED_OLD_ROOT_PRF)
        .await
        .unwrap();
    let unrelated_old_root_did = unrelated_old_root.did().to_string();
    let mut confirmation_args = BTreeMap::new();
    confirmation_args.insert(
        "oldRootDid".to_string(),
        Promised::String(unrelated_old_root_did),
    );
    let confirmation_bytes = root_container(
        new_root,
        vec!["account".into(), "recover".into(), "confirm".into()],
        confirmation_args,
    )
    .await;

    let response = client
        .post(format!("{base}/accounts/recover"))
        .json(&serde_json::json!({
            "recovery": hex::encode(recovery_bytes),
            "confirmation": hex::encode(confirmation_bytes),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // The account root did not change: a device-signed invocation under
    // the OLD root still succeeds.
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
}

#[dialog_common::test]
async fn it_rejects_a_replayed_recovery() {
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

    // POST /accounts -> create the account from a root-signed ceremony,
    // device A its first device.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_a = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
    let ceremony = tonk_identity::ceremony::create_account(
        root,
        "person@example.com".into(),
        code,
        "cred-1".into(),
        device_a.did(),
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

    // Link device B (the surviving device) from the root ceremony.
    let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let device_b = Ed25519Signer::import(&SURVIVING_DEVICE_SEED).await.unwrap();
    let ceremony = tonk_identity::ceremony::link_device(root, device_b.did(), "phone".into())
        .await
        .unwrap();
    let response = client
        .post(format!("{base}/devices/link"))
        .body(hex::decode(ceremony.invocation_hex).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Recover the account onto a fresh root, exactly as the happy path
    // does.
    let old_root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
        .await
        .unwrap();
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let old_root_did = old_root.did().to_string();
    let new_root_did = new_root.did().to_string();

    let device_link = tonk_identity::delegation::mint_device_delegation(old_root, &device_b.did())
        .await
        .unwrap();
    let fresh_delegation =
        tonk_identity::delegation::mint_device_delegation(new_root, &device_b.did())
            .await
            .unwrap();
    let device_delegation_hex = hex::encode(fresh_delegation.to_bytes().unwrap());

    let mut recovery_args = BTreeMap::new();
    recovery_args.insert(
        "newRootDid".to_string(),
        Promised::String(new_root_did.clone()),
    );
    recovery_args.insert(
        "newCredentialId".to_string(),
        Promised::String("cred-recovered".to_string()),
    );
    recovery_args.insert(
        "deviceDelegation".to_string(),
        Promised::String(device_delegation_hex.clone()),
    );
    let recovery_bytes = tonk_identity::request::build_device_invocation(
        device_b.clone(),
        &device_link,
        vec!["account".into(), "recover".into()],
        recovery_args,
    )
    .await
    .unwrap();

    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let mut confirmation_args = BTreeMap::new();
    confirmation_args.insert(
        "oldRootDid".to_string(),
        Promised::String(old_root_did.clone()),
    );
    let confirmation_bytes = root_container(
        new_root,
        vec!["account".into(), "recover".into(), "confirm".into()],
        confirmation_args,
    )
    .await;

    let response = client
        .post(format!("{base}/accounts/recover"))
        .json(&serde_json::json!({
            "recovery": hex::encode(&recovery_bytes),
            "confirmation": hex::encode(&confirmation_bytes),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Replay the identical {recovery, confirmation} container pair. The
    // recovery container's subject is still the OLD root, which no
    // longer resolves to any account -- the replay must be rejected.
    let response = client
        .post(format!("{base}/accounts/recover"))
        .json(&serde_json::json!({
            "recovery": hex::encode(&recovery_bytes),
            "confirmation": hex::encode(&confirmation_bytes),
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status() == 401 || response.status() == 403,
        "expected the replayed recovery to be rejected, got {}",
        response.status()
    );

    // The account is still on the NEW root -- not reverted or
    // corrupted: a device-signed call under the new root still
    // succeeds.
    let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
        .await
        .unwrap();
    let new_link = tonk_identity::delegation::mint_device_delegation(new_root, &device_b.did())
        .await
        .unwrap();
    let body = tonk_identity::request::build_device_invocation(
        device_b,
        &new_link,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await
    .unwrap();
    let response = client
        .post(format!("{base}/devices/list"))
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}
