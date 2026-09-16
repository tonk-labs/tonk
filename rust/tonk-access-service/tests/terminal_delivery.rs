//! Real HTTP delivery of signed complete approvals; no delivery row is authority.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::{Did, Principal};
use tonk_access_service::helpers::{AccessServer, AccessServiceAddress};
use tonk_invite::connection::{SpaceGrantBundle, candidate_build_scopes};
use tonk_invite::terminal::{Addition, Approval, LinkRequest, ReadAdditions, ReadRequest};

async fn bundle(
    account: &Ed25519Signer,
    browser: &Ed25519Signer,
    cli: &Did,
    device: &DelegationChain,
    remote: &url::Url,
    now: u64,
) -> anyhow::Result<SpaceGrantBundle> {
    let space = Ed25519Signer::generate().await?;
    let upstream = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&account.did())
        .subject(Subject::Specific(space.did()))
        .command(vec!["use".into()])
        .try_build()
        .await?;
    let scopes = candidate_build_scopes(&space.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        let leaf = DelegationBuilder::new()
            .issuer(Signer::from(browser.clone()))
            .audience(cli)
            .subject(Subject::Specific(space.did()))
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(Timestamp::try_from((now + 86400) as i128)?)
            .meta(std::collections::BTreeMap::from([(
                "home.address".into(),
                ipld_core::ipld::Ipld::String(remote.to_string()),
            )]))
            .try_build()
            .await?;
        chains.push(
            DelegationChain::new(upstream.clone())
                .push(device.proofs().next().unwrap().clone())?
                .push(leaf)?,
        );
    }
    SpaceGrantBundle::validate(
        chains,
        cli,
        &scopes,
        remote,
        Timestamp::try_from(now as i128)?,
    )
    .await
}

#[dialog_common::test]
async fn connection_delivery_http_authentication_replay_deadline_and_revoked_authority()
-> anyhow::Result<()> {
    let s3 = dialog_remote_s3::helpers::LocalS3::start_with_auth("test", "test", &["connections"])
        .await?;
    let server = AccessServer::start(s3, "connections", "test", "test", None, None, None).await?;
    let env = AccessServiceAddress {
        access_service_url: server.endpoint.clone(),
        s3_endpoint: server.s3_server.endpoint.clone(),
        bucket: "connections".into(),
        access_key_id: "test".into(),
        secret_access_key: "test".into(),
        service_did: server.service_did.clone(),
    };
    let base = url::Url::parse(&env.ucan_endpoint())?;
    let publish = base.join("/connection/delivery")?;
    let read_url = base.join("/connection/read")?;
    let client = reqwest::Client::new();
    let account = Ed25519Signer::generate().await?;
    env.activate_customer(&account, "terminal-delivery@example.test")
        .await?;
    let browser = Ed25519Signer::generate().await?;
    let cli = Ed25519Signer::generate().await?;
    let stranger = Ed25519Signer::generate().await?;
    let browser_signer = Signer::from(browser.clone());
    let cli_signer = Signer::from(cli.clone());
    let device =
        tonk_identity::delegation::mint_device_delegation(account.clone(), &browser.did()).await?;
    let service: Did = env.service_did.parse()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let request = LinkRequest::sign(
        &cli_signer,
        &service,
        [1; 32],
        now,
        now + 300,
        "terminal",
        Some(&account.did()),
    )
    .await?;
    let read = ReadRequest::sign(&cli_signer, &request.id(), [2; 32], now).await?;
    let post = |url: url::Url, body: Vec<u8>| {
        client
            .post(url)
            .header("Content-Type", "application/cbor")
            .body(body)
            .send()
    };
    assert_eq!(
        post(read_url.clone(), read.bytes().to_vec())
            .await?
            .status(),
        204
    );
    let grants = bundle(&account, &browser, &cli.did(), &device, &base, now).await?;
    let approval = Approval::sign(
        &browser_signer,
        &request,
        device.clone(),
        vec![grants.clone()],
        now,
    )
    .await?;
    let response = post(publish.clone(), approval.bytes().to_vec()).await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    anyhow::ensure!(
        status == 201,
        "publication {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&bytes)?["recorded"]
            .as_bool()
            .unwrap()
    );
    let replay = post(publish.clone(), approval.bytes().to_vec()).await?;
    assert_eq!(replay.status(), 200);
    assert!(
        !replay.json::<serde_json::Value>().await?["recorded"]
            .as_bool()
            .unwrap()
    );
    let other_read =
        ReadRequest::sign(&Signer::from(stranger), &request.id(), [3; 32], now).await?;
    assert_eq!(
        post(read_url.clone(), other_read.bytes().to_vec())
            .await?
            .status(),
        204
    );
    assert_eq!(
        post(read_url.clone(), read.bytes().to_vec())
            .await?
            .bytes()
            .await?
            .as_ref(),
        approval.bytes()
    );
    let conflicting =
        Approval::sign_decline(&browser_signer, &request, device.clone(), now).await?;
    assert_eq!(
        post(publish.clone(), conflicting.bytes().to_vec())
            .await?
            .status(),
        409
    );
    let old_request = LinkRequest::sign(
        &cli_signer,
        &service,
        [4; 32],
        now - 600,
        now - 300,
        "expired",
        Some(&account.did()),
    )
    .await?;
    let old_approval =
        Approval::sign_decline(&browser_signer, &old_request, device.clone(), now - 590).await?;
    assert_eq!(
        post(publish.clone(), old_approval.bytes().to_vec())
            .await?
            .status(),
        410
    );
    let mut tampered = read.bytes().to_vec();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert_eq!(post(read_url.clone(), tampered).await?.status(), 401);

    // A terminal can be offline while its browser adds another space. Delivery
    // remains usable after the original request deadline, with fresh authority.
    let addition_url = publish.join("/connection/addition")?;
    let additions_read_url = publish.join("/connection/additions/read")?;
    let short_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let short = LinkRequest::sign(
        &cli_signer,
        &service,
        [20; 32],
        short_now,
        short_now + 2,
        "offline terminal",
        None,
    )
    .await?;
    let initial = Approval::sign(
        &browser_signer,
        &short,
        device.clone(),
        vec![grants.clone()],
        short_now,
    )
    .await?;
    let initial_response = post(publish.clone(), initial.bytes().to_vec()).await?;
    assert_eq!(initial_response.status(), 201);
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let added_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    assert!(added_now >= short.deadline());
    let added_grants = bundle(&account, &browser, &cli.did(), &device, &base, added_now).await?;
    let addition = Addition::sign(
        &browser_signer,
        &Approval::inspect(initial.bytes()).await?,
        device.clone(),
        vec![added_grants.clone()],
        [21; 32],
        added_now,
    )
    .await?;
    assert_eq!(
        post(addition_url.clone(), addition.bytes().to_vec())
            .await?
            .status(),
        201
    );
    assert_eq!(
        post(addition_url.clone(), addition.bytes().to_vec())
            .await?
            .status(),
        200
    );
    let addition_read =
        ReadAdditions::sign(&cli_signer, &short.id(), 0, [22; 32], added_now).await?;
    let page: serde_json::Value = post(additions_read_url.clone(), addition_read.bytes().to_vec())
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(page["deliveries"].as_array().unwrap().len(), 1);
    assert_eq!(
        hex::decode(page["deliveries"][0]["bytes"].as_str().unwrap())?,
        addition.bytes()
    );
    let cursor = page["nextCursor"].as_u64().unwrap();
    let next_read =
        ReadAdditions::sign(&cli_signer, &short.id(), cursor, [23; 32], added_now).await?;
    let empty: serde_json::Value = post(additions_read_url.clone(), next_read.bytes().to_vec())
        .await?
        .json()
        .await?;
    assert!(empty["deliveries"].as_array().unwrap().is_empty());
    let stranger_signer = Signer::from(Ed25519Signer::generate().await?);
    let wrong_read =
        ReadAdditions::sign(&stranger_signer, &short.id(), 0, [24; 32], added_now).await?;
    let private: serde_json::Value = post(additions_read_url, wrong_read.bytes().to_vec())
        .await?
        .json()
        .await?;
    assert!(private["deliveries"].as_array().unwrap().is_empty());
    // A different account can sign a plausible initial decision for a public
    // request with no expected account, but cannot replace the stored pin.
    let other_account = Ed25519Signer::generate().await?;
    env.activate_customer(&other_account, "other-terminal-delivery@example.test")
        .await?;
    let other_browser = Ed25519Signer::generate().await?;
    let other_proof =
        tonk_identity::delegation::mint_device_delegation(other_account, &other_browser.did())
            .await?;
    let other_signer = Signer::from(other_browser);
    let fake_initial = Approval::sign(
        &other_signer,
        &short,
        other_proof.clone(),
        vec![grants.clone()],
        short_now,
    )
    .await?;
    let forged = Addition::sign(
        &other_signer,
        &fake_initial,
        other_proof,
        vec![added_grants.clone()],
        [25; 32],
        added_now,
    )
    .await?;
    assert_eq!(
        post(addition_url.clone(), forged.bytes().to_vec())
            .await?
            .status(),
        403
    );
    let revoke =
        tonk_identity::revocation::mint_root_revocation(account, &device, &device.proof_cids()[0])
            .await?;
    post(base, revoke).await?.error_for_status()?;
    let stale_addition = Addition::sign(
        &browser_signer,
        &initial,
        device.clone(),
        vec![added_grants],
        [26; 32],
        added_now,
    )
    .await?;
    assert_eq!(
        post(addition_url.clone(), stale_addition.bytes().to_vec())
            .await?
            .status(),
        401
    );
    assert_eq!(
        post(addition_url, addition.bytes().to_vec())
            .await?
            .status(),
        200
    );
    let next =
        LinkRequest::sign(&cli_signer, &service, [5; 32], now, now + 300, "new", None).await?;
    let stale = Approval::sign(&browser_signer, &next, device, vec![grants], now).await?;
    assert_eq!(
        post(publish.clone(), stale.bytes().to_vec())
            .await?
            .status(),
        401
    );
    assert_eq!(
        post(publish, approval.bytes().to_vec()).await?.status(),
        200,
        "immutable receipt retry needs no new write"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires freshly built production Worker; run scripts/test-terminal-delivery-worker.sh"]
async fn connection_delivery_worker_survives_d1_restart() -> anyhow::Result<()> {
    let shim = std::env::var("TONK_CONNECTION_WORKER_SHIM")?;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let state = tempfile::tempdir()?;
    let service = Ed25519Signer::import(&[0x5d; 32]).await?;
    let account = Ed25519Signer::generate().await?;
    let browser = Ed25519Signer::generate().await?;
    let cli = Ed25519Signer::generate().await?;
    let stranger = Ed25519Signer::generate().await?;
    let cli_signer = Signer::from(cli.clone());
    let browser_signer = Signer::from(browser.clone());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let device =
        tonk_identity::delegation::mint_device_delegation(account.clone(), &browser.did()).await?;
    let request = LinkRequest::sign(
        &cli_signer,
        &service.did(),
        [71; 32],
        now,
        now + 300,
        "terminal",
        None,
    )
    .await?;
    let grants = bundle(
        &account,
        &browser,
        &cli.did(),
        &device,
        &"http://localhost/ucan/".parse()?,
        now,
    )
    .await?;
    let approval = Approval::sign(
        &browser_signer,
        &request,
        device.clone(),
        vec![grants.clone()],
        now,
    )
    .await?;
    let wrong_read = ReadRequest::sign(
        &Signer::from(stranger.clone()),
        &request.id(),
        [73; 32],
        now,
    )
    .await?;
    let conflicting =
        Approval::sign_decline(&browser_signer, &request, device.clone(), now).await?;
    let expired_request = LinkRequest::sign(
        &cli_signer,
        &service.did(),
        [74; 32],
        now - 600,
        now - 300,
        "expired",
        None,
    )
    .await?;
    let expired =
        Approval::sign_decline(&browser_signer, &expired_request, device.clone(), now - 590)
            .await?;
    let next = LinkRequest::sign(
        &cli_signer,
        &service.did(),
        [75; 32],
        now,
        now + 300,
        "new",
        None,
    )
    .await?;
    let stale = Approval::sign(
        &browser_signer,
        &next,
        device.clone(),
        vec![grants.clone()],
        now,
    )
    .await?;
    let extra = bundle(
        &account,
        &browser,
        &cli.did(),
        &device,
        &"http://localhost/ucan/".parse()?,
        now,
    )
    .await?;
    let addition = Addition::sign(
        &browser_signer,
        &approval,
        device.clone(),
        vec![extra.clone()],
        [81; 32],
        now,
    )
    .await?;
    let stale_addition = Addition::sign(
        &browser_signer,
        &approval,
        device.clone(),
        vec![extra.clone()],
        [82; 32],
        now,
    )
    .await?;
    let wrong_addition_read =
        ReadAdditions::sign(&Signer::from(stranger), &request.id(), 0, [84; 32], now).await?;
    let other_account = Ed25519Signer::generate().await?;
    let other_account_did = other_account.did();
    let other_browser = Ed25519Signer::generate().await?;
    let other_device = tonk_identity::delegation::mint_device_delegation(
        other_account.clone(),
        &other_browser.did(),
    )
    .await?;
    let other_signer = Signer::from(other_browser);
    let fake_initial = Approval::sign(
        &other_signer,
        &request,
        other_device.clone(),
        vec![grants.clone()],
        now,
    )
    .await?;
    let forged_addition = Addition::sign(
        &other_signer,
        &fake_initial,
        other_device.clone(),
        vec![extra.clone()],
        [85; 32],
        now,
    )
    .await?;
    // Exceed D1's single-value limit with a valid complete signed payload.
    let mut large_bundles = Vec::new();
    let mut grant_bytes = 0;
    while grant_bytes < 2_050_000 {
        let next = bundle(
            &account,
            &browser,
            &cli.did(),
            &device,
            &"http://localhost/ucan/".parse()?,
            now,
        )
        .await?;
        for chain in next.chains() {
            grant_bytes += chain.to_bytes()?.len();
        }
        large_bundles.push(next);
    }
    let large_request = LinkRequest::sign(
        &cli_signer,
        &service.did(),
        [91; 32],
        now,
        now + 600,
        "large",
        None,
    )
    .await?;
    let large = Approval::sign(
        &browser_signer,
        &large_request,
        device.clone(),
        large_bundles.clone(),
        now,
    )
    .await?;
    anyhow::ensure!(large.bytes().len() > 2_000_000 && large.bytes().len() <= 4 * 1024 * 1024);
    let large_conflict =
        Approval::sign_decline(&browser_signer, &large_request, device.clone(), now).await?;
    let large_addition = Addition::sign(
        &browser_signer,
        &large,
        device.clone(),
        large_bundles,
        [92; 32],
        now,
    )
    .await?;
    anyhow::ensure!(large_addition.bytes().len() > 2_000_000);
    let failed_request = LinkRequest::sign(
        &cli_signer,
        &service.did(),
        [93; 32],
        now,
        now + 600,
        "failure",
        None,
    )
    .await?;
    let failed = Approval::sign(
        &browser_signer,
        &failed_request,
        device.clone(),
        large.bundles().to_vec(),
        now,
    )
    .await?;
    let keeper_request = LinkRequest::sign(
        &cli_signer,
        &service.did(),
        [94; 32],
        now,
        now + 600,
        "other account",
        None,
    )
    .await?;
    let keeper = Approval::sign(
        &other_signer,
        &keeper_request,
        other_device.clone(),
        vec![grants],
        now,
    )
    .await?;
    let keeper_addition = Addition::sign(
        &other_signer,
        &keeper,
        other_device,
        vec![extra],
        [95; 32],
        now,
    )
    .await?;
    let purge_invocation = dialog_ucan_core::InvocationBuilder::new()
        .issuer(Signer::from(account.clone()))
        .audience(&service.did())
        .subject(&account.did())
        .command(vec!["void".into(), "customer".into(), "purge".into()])
        .arguments(std::collections::BTreeMap::new())
        .proofs(vec![])
        .expiration(Timestamp::try_from((now + 600) as i128)?)
        .try_build()
        .await?;
    let purge =
        dialog_ucan_core::Container::new(vec![serde_ipld_dagcbor::to_vec(&purge_invocation)?])
            .into_bytes()?;
    let read_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let read = ReadRequest::sign(&cli_signer, &request.id(), [72; 32], read_now).await?;
    let addition_read =
        ReadAdditions::sign(&cli_signer, &request.id(), 0, [83; 32], read_now).await?;
    let large_read =
        ReadRequest::sign(&cli_signer, &large_request.id(), [96; 32], read_now).await?;
    let large_addition_read =
        ReadAdditions::sign(&cli_signer, &large_request.id(), 0, [97; 32], read_now).await?;
    let keeper_read =
        ReadRequest::sign(&cli_signer, &keeper_request.id(), [98; 32], read_now).await?;
    let keeper_addition_read =
        ReadAdditions::sign(&cli_signer, &keeper_request.id(), 0, [99; 32], read_now).await?;
    eprintln!(
        "large production fixture: {} spaces, {} approval bytes, {} addition bytes",
        large.bundles().len(),
        large.bytes().len(),
        large_addition.bytes().len()
    );
    let revocation = tonk_identity::revocation::mint_root_revocation(
        account.clone(),
        &device,
        &device.proof_cids()[0],
    )
    .await?;
    let fixture = state.path().join("fixture.json");
    std::fs::write(
        &fixture,
        serde_json::to_vec(&serde_json::json!({
            "account":account.did().to_string(),"requestId":request.id(),
            "approval":approval.bytes(),"read":read.bytes(),"wrongRead":wrong_read.bytes(),
            "conflicting":conflicting.bytes(),"expired":expired.bytes(),"stale":stale.bytes(),"revocation":revocation,
            "addition":addition.bytes(),"staleAddition":stale_addition.bytes(),"additionRead":addition_read.bytes(),
            "wrongAdditionRead":wrong_addition_read.bytes(),"forgedAddition":forged_addition.bytes(),
            "otherAccount":other_account_did.to_string(),
            "large":large.bytes(),"largeRead":large_read.bytes(),"largeId":large_request.id(),"largeConflict":large_conflict.bytes(),
            "largeAddition":large_addition.bytes(),"largeAdditionRead":large_addition_read.bytes(),
            "failed":failed.bytes(),"failedId":failed_request.id(),
            "keeper":keeper.bytes(),"keeperRead":keeper_read.bytes(),"keeperAddition":keeper_addition.bytes(),"keeperAdditionRead":keeper_addition_read.bytes(),
            "purge":purge,"revocationKey":format!("revoked/{}/{}",device.proof_cids()[0],account.did()),
        }))?,
    )?;
    let result = std::process::Command::new("node")
        .arg(root.join("scripts/terminal-delivery-worker.cjs"))
        .arg(shim)
        .arg(fixture)
        .arg(root)
        .arg(state.path())
        .status()?;
    anyhow::ensure!(
        result.success(),
        "production delivery harness failed: {result}"
    );
    Ok(())
}
