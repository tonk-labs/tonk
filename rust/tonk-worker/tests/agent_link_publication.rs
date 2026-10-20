#![cfg(all(feature = "native-host", not(target_arch = "wasm32")))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::Response,
};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, time::Timestamp};
use dialog_varsig::Principal;
use tonk_access_service::helpers::{AccessServiceSettings, access_service};
use tonk_invite::connection::{AgentInvite, candidate_build_scopes};
use tonk_worker::AgentLinkWorker;

struct Invocation {
    result: Result<serde_json::Value, String>,
    log: String,
    native_started: std::time::Instant,
    native_finished: std::time::Instant,
}

async fn invoke(
    directory: std::path::PathBuf,
    remote: url::Url,
    link: String,
    document: String,
) -> anyhow::Result<Invocation> {
    invoke_tool(directory, remote, link, document, "tonk_evaluate").await
}

async fn invoke_tool(
    directory: std::path::PathBuf,
    remote: url::Url,
    link: String,
    document: String,
    tool: &'static str,
) -> anyhow::Result<Invocation> {
    tokio::task::spawn_blocking(move || {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let native_started = std::time::Instant::now();
        let mut child = Command::new(env!("CARGO_BIN_EXE_tonk-agent-worker"))
            .arg(directory)
            .arg(remote.as_str())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&serde_json::json!({
            "tool": tool, "agent_link": link, "document": document
            }))?)?;
        let output = child.wait_with_output()?;
        let native_finished = std::time::Instant::now();
        anyhow::ensure!(output.status.success(), "native worker terminated");
        let log = String::from_utf8(output.stdout)?;
        let marked = log
            .lines()
            .find_map(|line| line.strip_prefix("TONK_AGENT_LINK_RESULT "))
            .ok_or_else(|| anyhow::anyhow!("native worker emitted no result"))?;
        let result: serde_json::Value = serde_json::from_str(marked)?;
        let result = match result.get("error").and_then(|error| error.as_str()) {
            Some(error) => Err(error.to_owned()),
            None => Ok(result["result"].clone()),
        };
        Ok(Invocation {
            result,
            log,
            native_started,
            native_finished,
        })
    })
    .await?
}

#[path = "agent_link_publication/reliability.rs"]
mod reliability;

#[derive(Clone)]
struct PublicationProxy {
    upstream: String,
    client: reqwest::Client,
    remaining: Arc<AtomicUsize>,
    refused: Arc<AtomicUsize>,
    unavailable: bool,
    lose_publication_response: bool,
    blocked: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<()>>,
}

async fn forward(
    State(proxy): State<PublicationProxy>,
    request: Request<Body>,
) -> Result<Response, StatusCode> {
    let (parts, body) = request.into_parts();
    // Only the synthetic writer's conditional content-head publication is
    // blocked. Uploads, pulls, authorization, and the independent rival use
    // their ordinary production paths.
    // Native replication uses direct UCAN requests: POST carries the signed
    // operation in Authorization and labels its command in the query. This
    // invitation permits cell publication only for branch/main/revision.
    let head_put = parts.method == "POST"
        && url::form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
            .any(|(key, value)| key == "cmd" && value == "/use/put/memory/cell");
    if head_put && proxy.unavailable {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if head_put
        && proxy
            .remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    {
        let (resume, wait) = tokio::sync::oneshot::channel();
        proxy
            .blocked
            .send(resume)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        wait.await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let body = axum::body::to_bytes(body, 4_000_000)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let response = proxy
        .client
        .request(
            parts.method.clone(),
            format!("{}{}", proxy.upstream, parts.uri),
        )
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let status = response.status();
    if head_put && status == StatusCode::PRECONDITION_FAILED {
        proxy.refused.fetch_add(1, Ordering::SeqCst);
    }
    let headers = response.headers().clone();
    let body = response
        .bytes()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .to_vec();
    if head_put && status.is_success() && proxy.lose_publication_response {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let mut output = Response::builder().status(status);
    for (name, value) in &headers {
        if name != "content-length" && name != "transfer-encoding" {
            output = output.header(name, value);
        }
    }
    output
        .body(Body::from(body))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn invitation(owner: &Signer, seed: [u8; 32], remote: &url::Url) -> anyhow::Result<String> {
    let recipient = Ed25519Signer::import(&seed).await?;
    let now = Timestamp::now();
    let deadline = Timestamp::try_from((now.to_unix() + 3600) as i128)?;
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        chains.push(DelegationChain::new(
            DelegationBuilder::new()
                .issuer(owner.clone())
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .expiration(deadline)
                .meta(tonk_invite::home_address_meta(remote))
                .try_build()
                .await?,
        ));
    }
    Ok(AgentInvite::new(seed, chains, &scopes, remote, now)
        .await?
        .to_url("https://tonk.example/agent")?)
}

#[tokio::test]
async fn independent_remote_burst_publishes_each_evaluation_once() -> anyhow::Result<()> {
    race(4, false, false, false).await
}

#[tokio::test]
async fn same_artifact_converges_without_replaying_evaluation() -> anyhow::Result<()> {
    race(1, true, false, false).await
}

#[tokio::test]
async fn sustained_contention_retains_original_without_replay() -> anyhow::Result<()> {
    race(usize::MAX, false, false, false).await
}

#[tokio::test]
async fn unavailable_publication_is_not_retried() -> anyhow::Result<()> {
    race(0, false, true, false).await
}

#[tokio::test]
async fn lost_publication_response_is_retained_without_retry() -> anyhow::Result<()> {
    race(3, false, false, true).await
}

async fn race(
    conflicts: usize,
    same_artifact: bool,
    unavailable: bool,
    lose_publication_response: bool,
) -> anyhow::Result<()> {
    let service = access_service(AccessServiceSettings::default()).await?;
    let owner = Signer::from(Ed25519Signer::generate().await?);
    service
        .address
        .provision_subject(owner.did().as_ref())
        .await?;
    let remote: url::Url = service.address.ucan_endpoint().parse()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let (blocked, mut blocked_rx) = tokio::sync::mpsc::unbounded_channel();
    let remaining = Arc::new(AtomicUsize::new(0));
    let refused = Arc::new(AtomicUsize::new(0));
    let proxy = PublicationProxy {
        upstream: service.address.access_service_url.clone(),
        client: reqwest::Client::new(),
        remaining: remaining.clone(),
        refused: refused.clone(),
        unavailable,
        lose_publication_response,
        blocked,
    };
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(forward).with_state(proxy))
            .await
            .unwrap();
    });
    let root =
        std::env::temp_dir().join(format!("tonk-agent-publication-{}", rand::random::<u64>()));
    let direct_link = invitation(&owner, [71; 32], &remote).await?;
    let proxy_remote: url::Url = format!("{origin}/ucan/").parse()?;
    let writer_link = invitation(&owner, [72; 32], &proxy_remote).await?;
    let mut rival = AgentLinkWorker::new(root.join("rival"), remote.clone());
    rival.call("tonk_evaluate", &direct_link,
        "concept!: &publication-record\n  with:\n    title: { the: example.publication/title, as: text, description: Test title }\n  maybe:\n    left: { the: example.publication/left, as: text, description: First writer value }\n    right: { the: example.publication/right, as: text, description: Rival writer value }\npublication-record!:\n  this: urn:publication:base\n  title: Base\n").await.map_err(anyhow::Error::msg)?;
    let mut writer = AgentLinkWorker::new(root.join("writer"), proxy_remote.clone());
    writer
        .call("tonk_query", &writer_link, "publication-record:\n")
        .await
        .map_err(anyhow::Error::msg)?;
    remaining.store(conflicts, Ordering::SeqCst);
    let document = if same_artifact {
        "publication-record!:\n  this: urn:publication:shared\n  title: Original evaluated once\n  left: Writer\n"
    } else {
        "publication-record!:\n  title: Original evaluated once\n"
    };
    let started = std::time::Instant::now();
    let write = invoke(
        root.join("writer"),
        proxy_remote,
        writer_link.clone(),
        // No explicit identity: replay would mint another visible instance.
        document.to_owned(),
    );
    tokio::pin!(write);
    let mut rivals = 0;
    let written = loop {
        tokio::select! {
            result = &mut write => break result,
            event = blocked_rx.recv() => {
                let resume = event.ok_or_else(|| anyhow::anyhow!("publication barrier closed"))?;
                let rival_identity = if conflicts == usize::MAX { 0 } else { rivals };
                let document = if same_artifact {
                    "publication-record!:\n  this: urn:publication:shared\n  title: Rival\n  right: Rival\n".to_owned()
                } else { format!("publication-record!:\n  this: urn:publication:rival:{rival_identity}\n  title: Rival {rivals}\n") };
                rival.call("tonk_evaluate", &direct_link, &document).await.map_err(anyhow::Error::msg)?;
                rivals += 1;
                resume.send(()).map_err(|_| anyhow::anyhow!("writer abandoned publication"))?;
            }
        }
    };
    eprintln!(
        "controlled remote publication refusals={}",
        refused.load(Ordering::SeqCst)
    );
    let result = async {
        let invocation = written?;
        anyhow::ensure!(
                invocation
                    .log
                    .lines()
                    .filter(|line| line.starts_with("evaluate repo=")
                        && line.ends_with("transact=true"))
                    .count()
                    == 1,
                "exactly one committing evaluation must be dispatched"
            );
        let attempts = invocation
            .log
            .lines()
            .filter(|line| line.starts_with("Push publication attempt "))
            .count();
        let retained = unavailable || lose_publication_response || conflicts == usize::MAX;
        if retained {
            let error = invocation.result.unwrap_err();
            anyhow::ensure!(
                error.starts_with("Applied locally; remote push could not be confirmed."),
                "unexpected publication error: {error}"
            );
            if conflicts == usize::MAX {
                anyhow::ensure!(error.ends_with("(HTTP 409; SYNC_CONFLICT)"));
                anyhow::ensure!(
                    started.elapsed() >= std::time::Duration::from_secs(30)
                        && started.elapsed() < std::time::Duration::from_secs(45),
                    "retry admission window must remain bounded"
                );
            } else {
                anyhow::ensure!(
                    attempts == conflicts + 1,
                    "unavailable/unknown publication must stop immediately"
                );
            }
            let retained = writer
                .call("tonk_query", &writer_link, "publication-record:\n")
                .await
                .map_err(anyhow::Error::msg)?;
            anyhow::ensure!(retained["recovery"]["required"] == true);
            anyhow::ensure!(
                retained["matches_before"][0]["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["fields"]["title"] == "Original evaluated once")
                    .count()
                    == 1
            );
            let refusal = writer
                .call("tonk_evaluate", &writer_link, document)
                .await
                .unwrap_err();
            anyhow::ensure!(
                refusal.starts_with("An earlier evaluation is unresolved."),
                "retained state must prevent reevaluation"
            );
        } else {
            invocation.result.map_err(anyhow::Error::msg)?;
        }
        anyhow::ensure!(
            refused.load(Ordering::SeqCst) == rivals,
            "every controlled conditional publication must have been refused"
        );
        let mut reader = AgentLinkWorker::new(root.join("reader"), remote);
        let read = reader
            .call("tonk_query", &direct_link, "publication-record:\n")
            .await
            .map_err(anyhow::Error::msg)?;
        let records = read["matches_before"][0]["results"].as_array().unwrap();
        if same_artifact {
            let shared = records
                .iter()
                .find(|r| r["this"] == "urn:publication:shared")
                .unwrap();
            anyhow::ensure!(
                shared["fields"]["left"] == "Writer" && shared["fields"]["right"] == "Rival",
                "both disjoint fields must survive"
            );
            anyhow::ensure!(
                ["Original evaluated once", "Rival"]
                    .contains(&shared["fields"]["title"].as_str().unwrap()),
                "shared field must select a submitted value"
            );
            let second = rival
                .call("tonk_query", &direct_link, "publication-record:\n")
                .await
                .map_err(anyhow::Error::msg)?;
            let other = second["matches_before"][0]["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["this"] == "urn:publication:shared")
                .unwrap();
            anyhow::ensure!(
                shared["fields"] == other["fields"],
                "independent replicas must converge"
            );
        } else {
            let original_count = usize::from(!retained || lose_publication_response);
            let rival_records = if conflicts == usize::MAX { 1 } else { rivals };
            anyhow::ensure!(
                records.len() == 1 + rival_records + original_count,
                "independent inventory must retain every published operation exactly once"
            );
            anyhow::ensure!(
                records
                    .iter()
                    .filter(|r| r["fields"]["title"] == "Original evaluated once")
                    .count()
                    == original_count,
                "evaluation must not be replayed"
            );
        }
        anyhow::ensure!(
            read.get("recovery").is_none(),
            "independent reader must not have pending publication"
        );
        anyhow::Ok(())
    }
    .await;
    server.abort();
    std::fs::remove_dir_all(root)?;
    result
}
