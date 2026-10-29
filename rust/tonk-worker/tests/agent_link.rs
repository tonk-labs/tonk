#![cfg(all(feature = "native-host", not(target_arch = "wasm32")))]

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, time::Timestamp};
use dialog_varsig::Principal;
use tonk_access_service::helpers::{AccessServiceSettings, access_service};
use tonk_invite::connection::{AgentInvite, candidate_build_scopes};
use tonk_worker::AgentLinkWorker;

#[tokio::test]
async fn malformed_evaluation_is_rejected_before_replica_or_journal_creation() -> anyhow::Result<()>
{
    let root = std::env::temp_dir().join(format!(
        "tonk-agent-invalid-notation-{}",
        rand::random::<u64>()
    ));
    let remote = "http://127.0.0.1:9/ucan/".parse()?;
    let owner = Signer::from(Ed25519Signer::generate().await?);
    let seed = [47; 32];
    let recipient = Ed25519Signer::import(&seed).await?;
    let now = Timestamp::now();
    let deadline = Timestamp::try_from((now.to_unix() + 3600) as i128)?;
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        let grant = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(deadline)
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(grant));
    }
    let link = AgentInvite::new(seed, chains, &scopes, &remote, now)
        .await?
        .to_url("https://tonk.example/agent")?;
    let mut worker = AgentLinkWorker::new(root.clone(), remote);
    // The failed live dossier used an unquoted colon-space in this scalar.
    // Reject the entire stream before opening storage, pulling, or journaling.
    let malformed = "relay-artifact!:\n  id: RES-SRC-D01\n  sources: RES-D01; ACC-D01; OPS-D01; scope boundary: same building access window; Alder only\n";
    let valid = "concept!: &valid-record\n  with:\n    title: { the: example.valid/title, as: text, description: Test title }\nvalid-record!:\n  title: Must not be committed from an invalid stream\n";
    for (document, line) in [
        (malformed.to_owned(), 3),
        (format!("{valid}---\n{malformed}"), 9),
    ] {
        let error = worker
            .call("tonk_evaluate", &link, &document)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            format!("Invalid notation (line {line}, column 53); no evaluation was dispatched.")
        );
        assert!(
            !root.exists(),
            "malformed notation must not create a replica or journal"
        );
    }
    Ok(())
}

#[tokio::test]
async fn agent_link_syncs_and_reads_back_from_an_independent_replica() -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let owner_key = Ed25519Signer::generate().await?;
    let owner = Signer::from(owner_key.clone());
    service
        .address
        .provision_subject(owner.did().as_ref())
        .await?;
    let remote = service.address.ucan_endpoint().parse()?;
    let seed = [51; 32];
    let recipient = Ed25519Signer::import(&seed).await?;
    let now = Timestamp::now();
    let deadline = Timestamp::try_from((now.to_unix() + 3600) as i128)?;
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        let grant = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(deadline)
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(grant));
    }
    let link = AgentInvite::new(seed, chains.clone(), &scopes, &remote, now)
        .await?
        .to_url("https://tonk.example/agent")?;
    let root =
        std::env::temp_dir().join(format!("tonk-agent-sync-spike-{}", rand::random::<u64>()));
    let mut writer = AgentLinkWorker::new(root.join("writer"), remote.clone());
    let written = writer.call("tonk_evaluate", &link,
        "concept!: &spike-record\n  with:\n    title: { the: example.spike/title, as: text, description: Test title }\nspike-record!:\n  this: urn:spike:remote\n  title: Synced through agent link\n").await
        .map_err(anyhow::Error::msg)?;
    assert_ne!(written["revision_before"], written["revision_after"]);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("writer/worker.lock"))?;
    lock.lock()?;
    let busy = writer
        .call("tonk_query", &link, "concept:\n")
        .await
        .unwrap_err();
    assert!(busy.contains("busy"));
    drop(lock);

    // Empty storage: this result must come from the remote, not the writer cache.
    let mut reader = AgentLinkWorker::new(root.join("reader"), remote.clone());
    let read = reader
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(read.to_string().contains("Synced through agent link"));
    assert_eq!(read["revision_before"], read["revision_after"]);
    assert!(!read.to_string().contains("tonk-agent-v2"));
    let config = serde_json::json!({
        "binary": env!("CARGO_BIN_EXE_tonk-agent-worker"),
        "directory": root.join("mcp"),
        "remote": remote,
        "link": link,
    });
    let output = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../mcp/integration/agent-link-client.mjs");
        let mut child = Command::new("node")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&config)?)?;
        Ok(child.wait_with_output()?)
    })
    .await??;
    anyhow::ensure!(
        output.status.success(),
        "MCP subprocess failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let read = reader
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(read.to_string().contains("Written through stdio MCP"));
    assert!(read.to_string().contains("Written through HTTP MCP"));

    writer
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    // A malformed document is rejected before a pending marker is created.
    // Correcting the syntax must permit a normal pull/evaluate/push afterward.
    let malformed = "spike-record!:\n  title: archive; scope boundary: same building\n";
    let rejected = writer
        .call("tonk_evaluate", &link, malformed)
        .await
        .unwrap_err();
    assert!(rejected.starts_with("Invalid notation (line 2, column "));
    assert!(rejected.ends_with("no evaluation was dispatched."));
    let valid_write_started = std::time::Instant::now();
    writer
        .call(
            "tonk_evaluate",
            &link,
            "spike-record!:\n  title: Valid publication after syntax rejection\n",
        )
        .await
        .map_err(anyhow::Error::msg)?;
    eprintln!(
        "agent-link valid write local elapsed_ms={:.3}",
        valid_write_started.elapsed().as_secs_f64() * 1000.0
    );
    let read = reader
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(
        read.to_string()
            .contains("Valid publication after syntax rejection")
    );
    assert!(read.get("recovery").is_none());

    // Valid YAML can still be rejected by analysis. This exact query-like
    // document from the live failure must not poison a later valid write.
    let before_rejection = writer
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    let rejected = writer
        .call("tonk_evaluate", &link, "tonk/repository: {name}")
        .await
        .unwrap_err();
    let expected = "Write validation failed; no evaluation was dispatched. (HTTP 400)";
    if rejected != expected {
        // The baseline intentionally fails here. Remove only this fixture's
        // own temporary data before returning its concrete mismatch.
        std::fs::remove_dir_all(&root)?;
        anyhow::bail!("semantic rejection mismatch: expected {expected}; got {rejected}");
    }
    let after_rejection = writer
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        after_rejection["revision_after"],
        before_rejection["revision_after"]
    );
    assert!(after_rejection.get("recovery").is_none());
    let corrected_write = writer
        .call(
            "tonk_evaluate",
            &link,
            "spike-record!:\n  title: Valid publication after semantic rejection\n",
        )
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        corrected_write["revision_before"], after_rejection["revision_after"],
        "validation must not commit before the actual write starts"
    );
    assert_ne!(
        corrected_write["revision_after"], corrected_write["revision_before"],
        "the corrected write must commit after validation"
    );
    let corrected = reader
        .call(
            "tonk_query",
            &link,
            "spike-record:\n  title: Valid publication after semantic rejection\n",
        )
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        corrected["matches_before"][0]["results"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the corrected publication must be independently readable exactly once"
    );
    assert!(corrected.get("recovery").is_none());

    // Simulate a restart after publication but before clearing the journal.
    let replica = std::fs::read_dir(root.join("writer"))?
        .filter_map(Result::ok)
        .find(|entry| entry.path().is_dir())
        .unwrap()
        .path();
    std::fs::write(replica.join("pending-evaluation"), b"")?;
    let retained = writer
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(retained["recovery"]["required"], true);
    assert!(
        writer
            .call("tonk_evaluate", &link, "concept:\n")
            .await
            .is_err()
    );
    writer
        .call("publish_retained", &link, "")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(!replica.join("pending-evaluation").exists());

    let past = Timestamp::try_from((now.to_unix() - 7200) as i128)?;
    let expired_at = Timestamp::try_from((now.to_unix() - 3600) as i128)?;
    let mut expired_chains = Vec::new();
    for scope in &scopes {
        expired_chains.push(DelegationChain::new(
            DelegationBuilder::new()
                .issuer(owner.clone())
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .expiration(expired_at)
                .meta(tonk_invite::home_address_meta(&remote))
                .try_build()
                .await?,
        ));
    }
    let expired = AgentInvite::new(seed, expired_chains, &scopes, &remote, past)
        .await?
        .to_url("https://tonk.example/agent")?;
    assert!(
        writer
            .call("tonk_query", &expired, "concept:\n")
            .await
            .unwrap_err()
            .contains("expired")
    );
    // Revoke only cell PUT: pulls still work, but a locally applied write cannot publish.
    let put = scopes
        .iter()
        .position(|scope| scope.command.to_string() == "/use/put/memory/cell")
        .unwrap();
    revoke(
        &service.address.ucan_endpoint(),
        owner_key.clone(),
        &chains[put],
    )
    .await?;
    let failed = writer
        .call(
            "tonk_evaluate",
            &link,
            "spike-record!:\n  this: urn:spike:retained\n  title: Retained after refused push\n",
        )
        .await
        .unwrap_err();
    assert!(failed.contains("push could not be confirmed"));
    let retained = writer
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(retained.to_string().contains("Retained after refused push"));
    assert_eq!(retained["recovery"]["required"], true);
    assert!(
        writer
            .call("tonk_evaluate", &link, "concept:\n")
            .await
            .is_err()
    );
    assert!(writer.call("publish_retained", &link, "").await.is_err());
    assert!(replica.join("pending-evaluation").exists());
    let remote_read = reader
        .call("tonk_query", &link, "spike-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(
        !remote_read
            .to_string()
            .contains("Retained after refused push")
    );
    // Revoked GET must deny even retained/cached content on the next invocation.
    let get = scopes
        .iter()
        .position(|scope| scope.command.to_string() == "/use/get/memory/cell")
        .unwrap();
    revoke(&service.address.ucan_endpoint(), owner_key, &chains[get]).await?;
    assert!(
        writer
            .call("tonk_query", &link, "spike-record:\n")
            .await
            .is_err()
    );
    assert!(
        reader
            .call("tonk_query", &link, "spike-record:\n")
            .await
            .is_err()
    );
    service.stop().await?;
    std::fs::remove_dir_all(root)?;
    Ok(())
}

async fn revoke(remote: &str, owner: Ed25519Signer, chain: &DelegationChain) -> anyhow::Result<()> {
    let leaf = chain.proof_cids().last().unwrap().to_owned();
    let bytes = tonk_identity::revocation::mint_root_revocation(owner, chain, &leaf).await?;
    let input = serde_json::to_vec(&serde_json::json!({"url": remote, "body": bytes}))?;
    let success = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("node").args(["--input-type=module", "-e",
            "let s=''; for await (const c of process.stdin) s+=c; const x=JSON.parse(s); const r=await fetch(x.url,{method:'POST',headers:{'content-type':'application/cbor'},body:Buffer.from(x.body)}); await r.body?.cancel(); process.exit(r.ok?0:1);"])
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        child.stdin.take().unwrap().write_all(&input)?;
        Ok(child.wait()?.success())
    }).await??;
    anyhow::ensure!(success, "fixture revocation failed");
    Ok(())
}

#[tokio::test]
async fn storage_admission_reports_safe_distinct_reasons_before_dispatch() -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let root = std::env::temp_dir().join(format!("tonk-agent-admission-{}", rand::random::<u64>()));
    let remote = "http://127.0.0.1:9/ucan/".parse()?;
    let owner = Signer::from(Ed25519Signer::generate().await?);
    let seed = [47; 32];
    let recipient = Ed25519Signer::import(&seed).await?;
    let now = Timestamp::now();
    let deadline = Timestamp::try_from((now.to_unix() + 3600) as i128)?;
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        let grant = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(deadline)
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(grant));
    }
    let link = AgentInvite::new(seed, chains, &scopes, &remote, now)
        .await?
        .to_url("https://tonk.example/agent")?;

    std::fs::DirBuilder::new().mode(0o700).create(&root)?;
    let capacity = root.join("file-capacity");
    std::fs::DirBuilder::new().mode(0o700).create(&capacity)?;
    // The lock itself is the 3501st file: a successful prior invocation may
    // have checkpointed more files than the next invocation admits.
    for i in 0..3500 {
        std::fs::File::create(capacity.join(i.to_string()))?;
    }
    let bytes = root.join("byte-capacity");
    std::fs::DirBuilder::new().mode(0o700).create(&bytes)?;
    std::fs::File::create(bytes.join("sparse"))?.set_len(192 * 1024 * 1024 + 1)?;
    let invalid = root.join("invalid");
    std::fs::create_dir(&invalid)?;
    std::fs::set_permissions(&invalid, std::fs::Permissions::from_mode(0o755))?;
    let busy = root.join("busy");
    std::fs::DirBuilder::new().mode(0o700).create(&busy)?;
    let lock = std::fs::File::create(busy.join("worker.lock"))?;
    lock.lock()?;
    let dangling = root.join("dangling-lock");
    std::fs::DirBuilder::new().mode(0o700).create(&dangling)?;
    let outside_lock = root.join("outside-lock-target");
    std::os::unix::fs::symlink(&outside_lock, dangling.join("worker.lock"))?;
    let unavailable_parent = root.join("unavailable");
    std::fs::File::create(&unavailable_parent)?;
    let unavailable = unavailable_parent.join("child");
    let mut unavailable_worker = AgentLinkWorker::new(unavailable.clone(), remote.clone());
    let unavailable_error = unavailable_worker
        .call("tonk_query", &link, "concept:\n")
        .await
        .unwrap_err();
    let mut mismatches = Vec::new();
    if unavailable_error != "Agent storage is unavailable; no evaluation was dispatched." {
        mismatches.push(format!(
            "unexpected I/O failure category: {unavailable_error:?}"
        ));
    }
    assert!(!unavailable.exists());
    for (directory, expected) in [
        (
            &capacity,
            "Agent storage file capacity reached; operator attention is required.",
        ),
        (
            &bytes,
            "Agent storage byte capacity reached; operator attention is required.",
        ),
        (
            &invalid,
            "Agent storage is invalid; operator attention is required.",
        ),
        (
            &busy,
            "Agent storage is busy; no evaluation was dispatched.",
        ),
        (
            &dangling,
            "Agent storage is invalid; operator attention is required.",
        ),
    ] {
        let mut worker = AgentLinkWorker::new(directory.clone(), remote.clone());
        let actual = worker
            .call("tonk_query", &link, "concept:\n")
            .await
            .unwrap_err();
        if actual != expected {
            mismatches.push(format!("expected {expected:?}, got {actual:?}"));
        }
        assert!(
            std::fs::read_dir(directory)?.all(|entry| !entry
                .unwrap()
                .file_type()
                .unwrap()
                .is_dir()),
            "admission failure must not open a replica"
        );
    }
    if outside_lock.exists() {
        mismatches.push("invalid lock opened its symlink target".to_owned());
    }
    drop(lock);
    std::fs::remove_dir_all(&root)?;
    anyhow::ensure!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    Ok(())
}

/// One `tonk-agent-worker` invocation with `input` on stdin, as the hosted
/// service makes it. Answers the marked result line's JSON.
fn invoke_agent_worker(root: &std::path::Path, input: serde_json::Value) -> serde_json::Value {
    use std::io::Write as _;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_tonk-agent-worker"))
        .arg(root)
        .arg("http://127.0.0.1:9/ucan/")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the agent worker starts");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let line = stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("TONK_AGENT_LINK_RESULT "))
        .expect("a marked result line");
    serde_json::from_str(line).unwrap()
}

/// A hosted agent keeps its key across processes, and a sponsorship issued
/// to that key by the sponsor's device is verified and kept.
#[tokio::test]
async fn hosted_agent_keeps_its_identity_and_its_sponsor_across_processes() -> anyhow::Result<()> {
    use tonk_invite::sponsorship::AgentSponsorship;

    let root = std::env::temp_dir().join(format!("tonk-hosted-agent-{}", rand::random::<u64>()));
    let created = invoke_agent_worker(&root, serde_json::json!({"tool": "agent_create"}));
    let agent = created["result"]["agent"].as_str().unwrap().to_owned();
    let did = created["result"]["did"].as_str().unwrap().to_owned();
    assert!(did.starts_with("did:key:z6Mk"));

    let identity = invoke_agent_worker(
        &root,
        serde_json::json!({"tool": "agent_identity", "agent": agent}),
    );
    assert_eq!(identity["result"]["did"], did);
    assert_eq!(identity["result"]["sponsor"], serde_json::Value::Null);

    // What the sponsor's device sends.
    let account = Ed25519Signer::generate().await?;
    let device = Ed25519Signer::generate().await?;
    let link = DelegationBuilder::new()
        .issuer(Signer::from(account.clone()))
        .audience(&device.did())
        .subject(dialog_ucan_core::subject::Subject::Any)
        .command(Vec::<String>::new())
        .try_build()
        .await?;
    let sponsorship = AgentSponsorship::issue_from_device(
        DelegationChain::new(link),
        &Signer::from(device),
        &did.parse()?,
        Timestamp::now(),
    )
    .await?;
    let recipient = "did:key:z6LSbysY2xFMRpGMhb7tFTLMpeuPRaqaWM1yECx2AtzE3KCc";
    let sponsored = invoke_agent_worker(
        &root,
        serde_json::json!({
            "tool": "agent_sponsor",
            "agent": agent,
            "sponsorship_hex": hex::encode(sponsorship.to_bytes()?),
            "recipient": recipient,
        }),
    );
    assert_eq!(sponsored["result"]["sponsor"], account.did().to_string());

    let identity = invoke_agent_worker(
        &root,
        serde_json::json!({"tool": "agent_identity", "agent": agent}),
    );
    assert_eq!(identity["result"]["did"], did);
    assert_eq!(identity["result"]["sponsor"], account.did().to_string());
    assert_eq!(identity["result"]["recipient"], recipient);

    // Link invocations keep their own shape: an agent field is not a link.
    let refused = invoke_agent_worker(
        &root,
        serde_json::json!({"tool": "tonk_query", "agent": agent}),
    );
    assert_eq!(refused["error"], "Tool unavailable.");
    Ok(())
}

/// A sponsored hosted agent and a space it created for its sponsor.
struct SponsoredSpace {
    service: dialog_common::helpers::Service<
        tonk_access_service::helpers::AccessServiceAddress,
        tonk_access_service::helpers::AccessServer,
    >,
    remote: url::Url,
    sponsor: Ed25519Signer,
    account_secret: tonk_identity::envelope::AccountSecret,
    root: std::path::PathBuf,
    agents: tonk_worker::HostedAgents,
    agent: String,
    space: serde_json::Value,
    space_did: dialog_varsig::Did,
}

async fn sponsored_space(name: &str) -> anyhow::Result<SponsoredSpace> {
    use tonk_invite::sponsorship::AgentSponsorship;
    use tonk_worker::HostedAgents;

    let service = access_service(AccessServiceSettings::default()).await?;
    let remote: url::Url = service.address.ucan_endpoint().parse()?;
    let sponsor = Ed25519Signer::generate().await?;
    service
        .address
        .activate_customer(&sponsor, "sponsor@example.com")
        .await?;
    let account_secret =
        tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([9u8; 32]));
    let recipient = account_secret.secret().did();

    let root = std::env::temp_dir().join(format!("tonk-sponsored-space-{}", rand::random::<u64>()));
    let agents = HostedAgents::new(root.join("agents"), remote.clone());
    let created = agents.create().await.map_err(anyhow::Error::msg)?;
    let agent = created["agent"].as_str().unwrap().to_owned();
    let agent_did: dialog_varsig::Did = created["did"].as_str().unwrap().parse()?;

    let device = Ed25519Signer::generate().await?;
    let link = DelegationBuilder::new()
        .issuer(Signer::from(sponsor.clone()))
        .audience(&device.did())
        .subject(dialog_ucan_core::subject::Subject::Any)
        .command(Vec::<String>::new())
        .try_build()
        .await?;
    let sponsorship = AgentSponsorship::issue_from_device(
        DelegationChain::new(link),
        &Signer::from(device),
        &agent_did,
        Timestamp::now(),
    )
    .await?;
    agents
        .sponsor(
            &agent,
            &hex::encode(sponsorship.to_bytes()?),
            recipient.as_ref(),
            Timestamp::now(),
        )
        .await
        .map_err(anyhow::Error::msg)?;

    let space = agents
        .create_space(&agent, name, "ChatGPT")
        .await
        .map_err(anyhow::Error::msg)?;
    let space_did: dialog_varsig::Did = space["space"].as_str().unwrap().parse()?;
    Ok(SponsoredSpace {
        service,
        remote,
        sponsor,
        account_secret,
        root,
        agents,
        agent,
        space,
        space_did,
    })
}

/// A sponsored hosted agent creates a space its sponsor owns: provisioned
/// under the sponsor, pushed, readable from an independent replica through
/// the founder chain in the claim link, with the space key sealed to the
/// sponsor's account in the link's fragment.
#[tokio::test]
async fn hosted_agent_creates_a_space_its_sponsor_owns() -> anyhow::Result<()> {
    let SponsoredSpace {
        service: _service,
        remote,
        sponsor,
        account_secret,
        root,
        agents,
        agent,
        space,
        space_did,
    } = sponsored_space("Notes from a chat").await?;
    let claim = url::Url::parse(space["claim_url"].as_str().unwrap())?;
    assert_eq!(claim.path(), "/join");

    // The sealed key opens only for the sponsor's account, and is the space's.
    let sealed = claim
        .fragment()
        .and_then(|fragment| fragment.strip_prefix("seed="))
        .expect("the claim link carries the sealed key");
    let sealed = tonk_identity::sealed::Sealed::decode(&bs58::decode(sealed).into_vec()?)?;
    let seed = account_secret.secret().reveal(&sealed, &space_did)?;
    assert_eq!(Ed25519Signer::import(&*seed).await?.did(), space_did);

    // The founder chain in the link is `space -> sponsor` with `/`.
    let access = claim
        .query_pairs()
        .find(|(key, _)| key == "access")
        .map(|(_, value)| value.into_owned())
        .expect("the claim link carries the founder chain");
    let founder = DelegationChain::try_from(bs58::decode(access).into_vec()?.as_slice())?;
    assert_eq!(founder.issuer(), &space_did);
    assert_eq!(founder.audience(), &sponsor.did());

    // The agent lists its space and writes to it through its own peer.
    let listed = agents
        .call_peer(&agent, "tonk_list_spaces", &serde_json::json!({}))
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(listed.to_string().contains(space_did.as_str()), "{listed}");
    agents
        .call_peer(
            &agent,
            "tonk_evaluate",
            &serde_json::json!({
                "space": space_did.to_string(),
                "document": "concept!: &chat-note\n  with:\n    text: { the: example.chat/text, as: text, description: Note text }\nchat-note!:\n  this: urn:chat:note\n  text: Remember the train times\n",
            }),
        )
        .await
        .map_err(anyhow::Error::msg)?;

    // The sponsor reads the space from the remote through that chain, from
    // empty storage: the agent provisioned and pushed it.
    let seed = [61; 32];
    let reader_key = Ed25519Signer::import(&seed).await?;
    let now = Timestamp::now();
    let deadline = Timestamp::try_from((now.to_unix() + 3600) as i128)?;
    let scopes = candidate_build_scopes(&space_did);
    let mut chains = Vec::new();
    for scope in &scopes {
        let leaf = DelegationBuilder::new()
            .issuer(Signer::from(sponsor.clone()))
            .audience(&reader_key.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(deadline)
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        chains.push(founder.clone().push(leaf)?);
    }
    let reader_link = AgentInvite::new(seed, chains, &scopes, &remote, now)
        .await?
        .to_url("https://tonk.example/agent")?;
    let mut reader = AgentLinkWorker::new(root.join("reader"), remote.clone());
    let read = reader
        .call("tonk_query", &reader_link, "tonk/repository:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(
        read.to_string().contains("Notes from a chat"),
        "the sponsor reads the agent's space: {read}"
    );
    let note = reader
        .call("tonk_query", &reader_link, "chat-note:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(
        note.to_string().contains("Remember the train times"),
        "the sponsor reads what the agent wrote: {note}"
    );
    Ok(())
}

/// A sponsored agent outlines the views its space nests: the to-do list
/// inside a trip is a node of its own, with a route to show it alone.
// Multi-threaded, as the agent worker runs: the outline runs on a blocking
// thread driven by the runtime handle.
#[tokio::test(flavor = "multi_thread")]
async fn hosted_agent_outlines_the_views_its_space_nests() -> anyhow::Result<()> {
    let SponsoredSpace {
        service: _service,
        agents,
        agent,
        space_did,
        ..
    } = sponsored_space("Trip planner").await?;
    let space = space_did.to_string();
    let call = |call: &'static str, arguments: serde_json::Value| {
        let mut arguments = arguments;
        arguments["space"] = serde_json::Value::String(space.clone());
        let agents = &agents;
        let agent = &agent;
        async move {
            agents
                .call_peer(agent, call, &arguments)
                .await
                .map_err(anyhow::Error::msg)
        }
    };
    call(
        "tonk_evaluate",
        serde_json::json!({
            "document": r#"concept!: &trip
  description: a trip being planned
  with:
    title: { the: example.trip/title, as: text, description: Trip title }
concept!: &todo
  description: something to do before leaving
  with:
    task: { the: example.todo/task, as: text, description: What to do }
view!:
  this: trip
  show:
    ui: "<section><h1>{title}</h1><tonk-display model=\"todo\"></tonk-display></section>"
view!:
  this: todo
  show:
    directory: "<ul><li>{task}</li></ul>"
trip!:
  this: urn:trip:lisbon
  title: Lisbon
todo!:
  this: urn:todo:flights
  task: Book flights
todo!:
  this: urn:todo:pack
  task: Pack
"#,
        }),
    )
    .await?;

    // The home outlines without naming a route.
    let home = call("tonk_outline", serde_json::json!({})).await?;
    assert_eq!(home["outline"]["model"], "tonk/space", "{home}");
    assert_eq!(home["outline"]["concept"], "tonk:blank", "{home}");
    assert!(home["outline"]["error"].is_null(), "{home}");

    let trip = call(
        "tonk_outline",
        serde_json::json!({ "route": "urn:trip:lisbon@trip" }),
    )
    .await?;
    let todo = &trip["outline"]["children"][0];
    assert_eq!(todo["route"], "todo", "{trip}");
    assert_eq!(todo["records"], 2, "{trip}");

    Ok(())
}

/// Serve mode holds a sponsored agent's directory open: it serves
/// the agent's worker on loopback only to a request carrying the host token,
/// and runs the agent's tool calls from stdin, on the same peer, so a write
/// through the served worker is what the next call reads.
#[tokio::test(flavor = "multi_thread")]
async fn a_served_agent_shares_one_peer_between_its_views_and_its_calls() -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let SponsoredSpace {
        service: _service,
        root,
        agents,
        agent,
        space_did,
        ..
    } = sponsored_space("Groceries").await?;
    let space = space_did.to_string();

    let live = root.join("agents").join(&agent);
    let token = "t".repeat(43);
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_tonk-agent-worker"))
        .arg(&live)
        .arg("http://127.0.0.1:9/ucan/")
        .arg("serve")
        .env("TONK_WORKER_HOST_TOKEN", &token)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    async fn marked(
        lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
        marker: &str,
    ) -> serde_json::Value {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("the server stopped");
            if let Some(json) = line.strip_prefix(marker) {
                return serde_json::from_str(json).unwrap();
            }
        }
    }
    let served = marked(&mut stdout, "TONK_AGENT_SERVE ").await;
    let origin = format!("http://{}", served["address"].as_str().expect("an address"));
    let base = format!("{origin}/api/repository/{space}/branch/main");

    let client = reqwest::Client::new();
    let refused = client
        .post(format!("{base}/evaluate?transact=false"))
        .body("tonk/repository:\n")
        .send()
        .await?;
    assert_eq!(refused.status(), 401);

    let written = client
        .post(format!("{base}/evaluate?transact=true"))
        .bearer_auth(&token)
        .header("content-type", "text/plain")
        .body("concept!: &grocery\n  with:\n    item: { the: example.grocery/item, as: text, description: Item }\ngrocery!:\n  this: urn:grocery:milk\n  item: Milk\n")
        .send()
        .await?;
    assert!(written.status().is_success(), "{}", written.text().await?);

    let call = serde_json::json!({
        "id": 7,
        "call": "tonk_query",
        "arguments": { "space": space, "document": "grocery:\n" },
    });
    stdin.write_all(format!("{call}\n").as_bytes()).await?;
    let reply = marked(&mut stdout, "TONK_AGENT_CALL ").await;
    assert_eq!(reply["id"], 7);
    assert!(reply["result"].to_string().contains("Milk"), "{reply}");

    // The served agent holds its directory: no one-shot process opens it.
    let busy = agents
        .call_peer(&agent, "tonk_list_spaces", &serde_json::json!({}))
        .await;
    assert_eq!(busy.unwrap_err(), "The agent is busy.");

    // A call outside the agent's space tools is refused.
    let refused = serde_json::json!({ "id": 8, "call": "agent_create" });
    stdin.write_all(format!("{refused}\n").as_bytes()).await?;
    assert_eq!(
        marked(&mut stdout, "TONK_AGENT_CALL ").await["error"],
        "Tool unavailable."
    );

    // Closing stdin stops the server.
    drop(stdin);
    assert!(child.wait().await?.success());
    Ok(())
}
