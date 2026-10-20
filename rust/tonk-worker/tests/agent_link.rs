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
