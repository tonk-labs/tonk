//! Opt-in, measured native reliability receipts; no models or live service.
use super::{invitation, invoke, invoke_tool};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_varsig::Principal;
use serde_json::{Value, json};
use sha2_0_10::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tonk_access_service::helpers::{AccessServiceSettings, access_service};

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn ms(clock: Instant) -> f64 {
    clock.elapsed().as_secs_f64() * 1000.0
}
fn at(clock: Instant, instant: Instant) -> f64 {
    instant.duration_since(clock).as_secs_f64() * 1000.0
}

fn retain(root: &Path, name: &str, content: &[u8]) -> anyhow::Result<Value> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(name))?;
    file.write_all(content)?;
    Ok(json!({"id": name, "path": name, "sha256": hash(content)}))
}

fn inventory(read: &Value, operations: &[Value]) -> anyhow::Result<Vec<Value>> {
    let rows = read["matches_before"][0]["results"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing inventory"))?;
    Ok(rows
        .iter()
        .map(|row| {
            let fields = operations
                .iter()
                .enumerate()
                .filter_map(|(i, op)| {
                    row["fields"]
                        .get(format!("op-{i}"))
                        .map(|value| (op["field"].as_str().unwrap().to_owned(), value.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            let mut record = json!({"artifact_id": row["fields"]["artifact-id"], "fields": fields});
            if let Some(shared) = row["fields"].get("shared") {
                record["shared_value"] = shared.clone();
            }
            record
        })
        .collect())
}

/// Run only on request, into a new private directory. The protocol plan is an
/// input, and all raw worker logs and observations survive unsuccessful cases.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires TONK_RELIABILITY_PLAN and a fresh TONK_RELIABILITY_OUTPUT directory"]
async fn emit_native_reliability_receipts() -> anyhow::Result<()> {
    let plan: Value =
        serde_json::from_slice(&std::fs::read(std::env::var("TONK_RELIABILITY_PLAN")?)?)?;
    let output = PathBuf::from(std::env::var("TONK_RELIABILITY_OUTPUT")?);
    std::fs::create_dir(&output)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700))?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = |args: &[&str]| -> anyhow::Result<Vec<u8>> {
        let result = std::process::Command::new("git")
            .args(args)
            .current_dir(&source)
            .output()?;
        anyhow::ensure!(result.status.success(), "source identity inspection failed");
        Ok(result.stdout)
    };
    let sha = String::from_utf8(git(&["rev-parse", "HEAD"])?)?
        .trim()
        .to_owned();
    let mut patch = git(&["diff", "--binary", "HEAD"])?;
    for name in String::from_utf8(git(&["ls-files", "--others", "--exclude-standard"])?)?.lines() {
        patch.extend_from_slice(format!("\nuntracked:{name}\n").as_bytes());
        patch.extend_from_slice(&std::fs::read(source.join(name))?);
    }
    let dirty = !patch.is_empty();
    let patch_hash = hash(&patch);
    let binary = env!("CARGO_BIN_EXE_tonk-agent-worker");
    let mut evidence = vec![
        retain(&output, "source.patch", &patch)?,
        retain(
            &output,
            "build.json",
            &serde_json::to_vec_pretty(&json!({
                "source_sha": sha, "source_dirty": dirty, "source_patch_sha256": patch_hash,
                "native_binary": binary, "native_binary_sha256": hash(&std::fs::read(binary)?)
            }))?,
        )?,
    ];
    let mut cases = Vec::new();
    for case in plan["cases"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing cases"))?
    {
        // The helper provisions one synthetic customer per service. Separate
        // services also keep each registered case's storage and authority fresh.
        let service = access_service(AccessServiceSettings::default()).await?;
        let remote: url::Url = service.address.ucan_endpoint().parse()?;
        let case_id = case["id"].as_str().unwrap();
        let operations = case["operations"].as_array().unwrap().clone();
        let state = output.join(format!("{case_id}-state"));
        std::fs::create_dir(&state)?;
        let owner = Signer::from(Ed25519Signer::generate().await?);
        let repository = owner.did().to_string();
        service.address.provision_subject(&repository).await?;
        let setup_link = invitation(&owner, [78; 32], &remote).await?;
        let mut schema = "concept!: &reliability-record\n  with:\n    artifact-id: { the: example.reliability/artifact-id, as: text, description: Registered artifact identity }\n  maybe:\n    shared: { the: example.reliability/shared, as: text, description: Concurrent shared value }\n".to_owned();
        for i in 0..operations.len() {
            schema.push_str(&format!(
                "    op-{i}: {{ the: example.reliability/op-{i}, as: text, description: Registered operation payload }}\n"
            ));
        }
        let setup = invoke(
            state.join("setup"),
            remote.clone(),
            setup_link.clone(),
            schema,
        )
        .await?;
        setup.result.map_err(anyhow::Error::msg)?;
        let initial = invoke_tool(
            state.join("initial"),
            remote.clone(),
            setup_link.clone(),
            "reliability-record:\n".to_owned(),
            "tonk_query",
        )
        .await?;
        let initial_records = inventory(&initial.result.map_err(anyhow::Error::msg)?, &operations)?;
        let mut case_log = format!(
            "repository={repository}\nsetup:\n{}\ninitial:\n{}\n",
            setup.log, initial.log
        );
        let clock = Instant::now();
        let mut links = Vec::new();
        let mut queues = Vec::new();
        for writer in 0..case["writers"].as_u64().unwrap() {
            links.push(invitation(&owner, [80 + writer as u8; 32], &remote).await?);
            queues.push(Arc::new(tokio::sync::Mutex::new(())));
        }
        let mut handles = Vec::new();
        let mut barriers = Vec::new();
        let mut schedule_origin = None;
        for round in 0..case["rounds"].as_u64().unwrap() {
            let offset = round * case["round_interval_ms"].as_u64().unwrap();
            let scheduled =
                schedule_origin.unwrap_or(clock) + std::time::Duration::from_millis(offset);
            if let Some(delay) = scheduled.checked_duration_since(Instant::now()) {
                tokio::time::sleep(delay).await;
            }
            let barrier = Arc::new(tokio::sync::Barrier::new(
                case["writers"].as_u64().unwrap() as usize + 1,
            ));
            let (ready_tx, mut ready_rx) = tokio::sync::mpsc::unbounded_channel();
            for (index, operation) in operations
                .iter()
                .enumerate()
                .filter(|(_, op)| op["round"].as_u64() == Some(round))
            {
                let operation = operation.clone();
                let writer = operation["writer"].as_u64().unwrap() as usize;
                let queue = queues[writer].clone();
                let link = links[writer].clone();
                let remote = remote.clone();
                let root = state.clone();
                let barrier = barrier.clone();
                let ready_tx = ready_tx.clone();
                let read_link = setup_link.clone();
                let out = output.clone();
                let id = case_id.to_owned();
                handles.push(tokio::spawn(async move {
                    ready_tx.send((writer.to_string(), ms(clock)))?;
                    barrier.wait().await;
                    let started = ms(clock); // Admission includes time in this writer's owner queue.
                    let lock = queue.lock().await;
                    let artifact = operation["artifact_id"].as_str().unwrap();
                    let entity = format!("urn:reliability:{}", hash(artifact.as_bytes()));
                    let mut document = format!("reliability-record!:\n  this: {entity}\n  artifact-id: {}\n  op-{index}: {}\n", serde_json::to_string(artifact)?, serde_json::to_string(&operation["payload_sha256"])?);
                    if !operation["shared_value"].is_null() { document.push_str(&format!("  shared: {}\n", serde_json::to_string(&operation["shared_value"])?)); }
                    let written = invoke(root.join(format!("writer-{writer}")), remote.clone(), link, document).await?;
                    let native_started = at(clock, written.native_started);
                    let native_finished = at(clock, written.native_finished);
                    drop(lock);
                    let evaluations = written.log.lines().filter(|line| line.starts_with("evaluate repo=") && line.ends_with("transact=true")).count();
                    let attempts = written.log.lines().filter(|line| line.starts_with("Push publication attempt ")).count();
                    let acknowledged = written.result.is_ok();
                    let published = acknowledged.then_some(native_finished);
                    let required = !acknowledged && evaluations != 0;
                    let read = invoke_tool(root.join(format!("readback-{index}")), remote, read_link, "reliability-record:\n".to_owned(), "tonk_query").await?;
                    let readback_at = at(clock, read.native_finished);
                    let matched = read.result.as_ref().ok().and_then(|value| value["matches_before"][0]["results"].as_array()).is_some_and(|rows| rows.iter().any(|r| r["this"] == entity && r["fields"][format!("op-{index}")] == operation["payload_sha256"]));
                    let status = if matched { "published_verified" } else if written.result.as_ref().err().is_some_and(|e| e.contains("SYNC_CONFLICT")) { "conflict" } else if required { "uncertain" } else { "failed" };
                    let evidence_name = format!("{id}-r{round:02}-w{writer:02}.log");
                    let raw = format!("operation={}\nstarted_ms={started}\nnative_execution_started_ms={native_started}\nnative_execution_finished_ms={native_finished}\nreadback_ms={readback_at}\nwrite:\n{}\nindependent readback:\n{}\n", operation["id"], written.log, read.log);
                    let artifact = retain(&out, &evidence_name, raw.as_bytes())?;
                    let finished = ms(clock);
                    let mut recovery = json!({"required": required, "resolution": if required { if matched { "readback" } else { "unresolved" } } else { "not_needed" }});
                    if required { recovery["required_ms"] = json!(native_finished); if matched { recovery["resolved_ms"] = json!(readback_at); } }
                    Ok::<_, anyhow::Error>((json!({"operation_id": operation["id"], "status": status, "evidence_id": evidence_name,
                        "started_ms": started, "published_ms": published, "readback_ms": matched.then_some(readback_at), "finished_ms": finished,
                        "native_execution_started_ms": native_started, "native_execution_finished_ms": native_finished,
                        "publication_attempts": attempts, "local_evaluation_count": evaluations, "recovery": recovery,
                        "readback": if matched { json!({"artifact_id": operation["artifact_id"], "field": operation["field"], "payload_sha256": operation["payload_sha256"]}) } else { Value::Null }
                    }), artifact))
                }));
            }
            let mut ready = BTreeMap::new();
            for _ in 0..case["writers"].as_u64().unwrap() {
                let (writer, time) = ready_rx.recv().await.unwrap();
                ready.insert(writer, time);
            }
            let release_instant = Instant::now();
            if round == 0 {
                schedule_origin = Some(release_instant);
            }
            let released = at(clock, release_instant);
            let record = json!({"round": round, "ready_ms": ready, "released_ms": released, "evidence_id": format!("{case_id}.log")});
            case_log.push_str(&format!("barrier={}\n", record));
            barriers.push(record);
            barrier.wait().await;
        }
        let mut receipts = Vec::new();
        for handle in handles {
            let (receipt, artifact) = handle.await??;
            receipts.push(receipt);
            evidence.push(artifact);
        }
        let mut final_records = Vec::new();
        let mut observed = 0.0;
        let mut independent = Vec::new();
        for observer in 0..2 {
            let root = state.join(format!("final-observer-{observer}"));
            let seed = [90 + observer; 32];
            let observer_link = invitation(&owner, seed, &remote).await?;
            let replica_id = Ed25519Signer::import(&seed).await?.did().to_string();
            let read = invoke_tool(
                root.clone(),
                remote.clone(),
                observer_link,
                "reliability-record:\n".to_owned(),
                "tonk_query",
            )
            .await?;
            observed = at(clock, read.native_finished);
            let records = inventory(&read.result.map_err(anyhow::Error::msg)?, &operations)?;
            case_log.push_str(&format!("observer={observer} registry_root={} replica_id={replica_id} observed_ms={observed}\n{}\n", root.display(), read.log));
            if case["artifact_scope"] == "case" {
                independent.push(json!({"observer_id": format!("observer-{observer}"), "replica_id": replica_id, "registry_root": root,
                    "repository": repository, "synced_ms": observed, "read_at_ms": observed, "evidence_id": format!("{case_id}.log"),
                    "shared_value": records.first().and_then(|r| r.get("shared_value"))}));
            }
            if observer == 0 {
                final_records = records;
            }
        }
        evidence.push(retain(
            &output,
            &format!("{case_id}.log"),
            case_log.as_bytes(),
        )?);
        let mut target = json!({"repository": repository, "build_sha": sha});
        if dirty {
            target["source_patch_sha256"] = json!(patch_hash);
        }
        cases.push(json!({"id": case_id, "target_identity": target, "evidence_id": format!("{case_id}.log"), "barriers": barriers,
            "initial_inventory_complete": true, "initial_inventory": initial_records,
            "receipts": receipts, "final_inventory_complete": true, "final_inventory": final_records,
            "final_observed_ms": observed, "independent_shared_readbacks": independent}));
    }
    let mut identity = json!({"command": ["cargo", "test", "-p", "tonk-worker", "--features", "native-host", "--test", "agent_link_publication", "reliability::emit_native_reliability_receipts", "--", "--ignored", "--exact", "--nocapture"],
        "source_sha": sha, "source_dirty": dirty,
        "boundary_detail": "Two independent native writer identities and roots. Each owner queue serializes its writer's processes. Admission timestamps include queue time; native execution timestamps are separately measured. Fresh native processes pull independent readbacks. Disposable local access service only."});
    if dirty {
        identity["source_patch_sha256"] = json!(patch_hash);
    }
    let mut canonical_plan = plan.clone();
    canonical_plan.sort_all_objects();
    let observations = json!({"plan_sha256": hash(&serde_json::to_vec(&canonical_plan)?), "boundary": "native_integration", "adapter_identity": identity, "evidence": evidence, "cases": cases});
    retain(
        &output,
        "observations.json",
        &serde_json::to_vec_pretty(&observations)?,
    )?;
    eprintln!(
        "Native reliability observations saved to {}",
        output.join("observations.json").display()
    );
    Ok(())
}
