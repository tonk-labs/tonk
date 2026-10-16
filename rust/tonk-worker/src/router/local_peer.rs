//! Native local peer, not a public HTTP route. Unlike the agent-link adapter,
//! the peer keeps its own long-lived identity: one profile, whose key it
//! generates on first use and never receives from a link. Pasted share links
//! are redeemed through the ordinary join, which delegates the membership to
//! this peer's account, so the link itself is no longer needed afterwards and
//! the space owner can withdraw it.
//!
//! The host pins the trusted access-service URL. A link naming any other
//! service is refused before anything is claimed, so content fetched by the
//! agent cannot point the peer at an unknown remote.
use std::path::{Path, PathBuf};

use axum::{Router, body::Body, http::Request};
use serde_json::{Value, json};
use tonk_invite::{Invite, Ticket};
use tower_service::Service;

use super::agent_link::{clear_journal, create_journal, request, request_method};
use crate::TonkState;

/// A persistent peer over one private data directory.
pub struct LocalPeerWorker {
    root: PathBuf,
    remote: url::Url,
}

impl LocalPeerWorker {
    /// Pin a host-owned data directory and independently trusted remote.
    pub fn new(root: PathBuf, remote: url::Url) -> Self {
        Self { root, remote }
    }

    /// Run one tool. Arguments are tool-specific; errors exclude upstream
    /// details and never echo a link.
    pub async fn call(&mut self, tool: &str, arguments: &Value) -> Result<Value, String> {
        self.execute(tool, arguments, true).await
    }

    async fn execute(
        &mut self,
        tool: &str,
        arguments: &Value,
        synchronize: bool,
    ) -> Result<Value, String> {
        let text = |name: &str| arguments.get(name).and_then(Value::as_str).unwrap_or("");
        match tool {
            "tonk_join" => {
                let link = text("link").trim();
                if link.is_empty() || link.len() > 160_000 {
                    return Err("Provide one complete Tonk share link.".into());
                }
                self.pin(link).await?;
                self.with_state(|router, _| async move { join(router, link).await })
                    .await
            }
            "tonk_list_spaces" => {
                self.with_state(|router, did| async move { list(router, did).await })
                    .await
            }
            "tonk_query" | "tonk_evaluate" | "publish_retained" => {
                let writing = tool == "tonk_evaluate";
                let recovering = tool == "publish_retained";
                let space = text("space");
                let document = text("document");
                if !space.starts_with("did:key:") || space.len() > 256 {
                    return Err("Provide the exact space subject from tonk_list_spaces.".into());
                }
                if !recovering && (document.trim().is_empty() || document.len() > 32_000) {
                    return Err("Provide one inline notation document, up to 32 KB.".into());
                }
                let journal = self.root.join("pending").join(space.replace(':', "_"));
                self.with_state(|router, did| async move {
                    if !list(router, did).await?["spaces"]
                        .as_array()
                        .is_some_and(|spaces| spaces.iter().any(|entry| entry["subject"] == space))
                    {
                        return Err(
                            "This peer has not joined that space. Ask for a share link and call tonk_join."
                                .to_owned(),
                        );
                    }
                    evaluate(router, space, document, &journal, writing, recovering, synchronize)
                        .await
                })
                .await
            }
            _ => Err("Tool unavailable.".into()),
        }
    }

    /// Refuse a link whose access service is not the pinned one.
    async fn pin(&self, link: &str) -> Result<(), String> {
        let malformed = || "That is not a Tonk share link.".to_owned();
        let remote = if let Some(ticket) = Ticket::public_for_url(link).map_err(|_| malformed())? {
            Some(ticket.remote().clone())
        } else if let Some(ticket) = Ticket::parse_url(link).map_err(|_| malformed())? {
            Some(ticket.remote().clone())
        } else {
            Invite::parse_url(link)
                .await
                .map_err(|_| malformed())?
                .remote_url
        };
        match remote {
            Some(remote) if remote.origin() == self.remote.origin() => Ok(()),
            _ => Err(format!(
                "This link belongs to a different Tonk service than this peer trusts ({}).",
                self.remote.origin().ascii_serialization()
            )),
        }
    }

    /// Open the one persistent profile under an exclusive lock, run `body`
    /// against its ordinary router, and shut everything down afterwards.
    async fn with_state<F, Fut>(&mut self, body: F) -> Result<Value, String>
    where
        F: FnOnce(&'static mut Router, String) -> Fut,
        Fut: std::future::Future<Output = Result<Value, String>>,
    {
        let _lock =
            lock_root(&self.root).map_err(|_| "Peer storage is unavailable or busy.".to_owned())?;
        let state = TonkState::open_native(&self.root.join("peer"))
            .await
            .map_err(|_| "Could not open the local peer.".to_owned())?;
        let did = state.profile.did().to_string();
        let (router, state, hub) = crate::api_router_with_state(state);
        // The router lives for exactly this call; leaking it keeps the borrow
        // simple for the closure and is reclaimed when the process exits.
        let router: &'static mut Router = Box::leak(Box::new(router));
        let result = body(router, did).await;
        state.read().await.shutdown();
        hub.shutdown().await;
        result
    }
}

async fn join(router: &mut Router, link: &str) -> Result<Value, String> {
    let body = json!({ "url": link }).to_string();
    let (ok, value) = send(router, "POST", "/api/profile/join", "application/json", body).await?;
    if !ok {
        // Surface the worker's classified reason (revoked, expired, private…)
        // but never anything that could contain the link.
        let reason = value
            .get("error")
            .and_then(Value::as_str)
            .filter(|reason| !reason.contains("://") && !reason.contains('#'))
            .unwrap_or("the space refused the link");
        return Err(format!("Could not join: {reason}."));
    }
    let repository = &value["repository"];
    Ok(json!({
        "subject": repository["subject"],
        "name": repository["label"],
        "renewed": value["outcome"] == "renewed",
    }))
}

async fn list(router: &mut Router, peer: String) -> Result<Value, String> {
    let failed = || "Could not read this peer's spaces.".to_owned();
    let profile = request_method(router, "/api/profile", "", "GET")
        .await
        .map_err(|_| failed())?;
    let mut spaces = Vec::new();
    for entry in profile["space"].as_array().into_iter().flatten() {
        let key = entry["key"].as_str().unwrap_or_default();
        let label = request_method(router, &format!("/api/repository/{key}"), "", "GET")
            .await
            .ok()
            .and_then(|info| info["label"].as_str().map(str::to_owned));
        spaces.push(json!({ "subject": entry["subject"], "name": label }));
    }
    Ok(json!({ "peer": peer, "spaces": spaces }))
}

async fn evaluate(
    router: &mut Router,
    space: &str,
    document: &str,
    journal: &Path,
    writing: bool,
    recovering: bool,
    synchronize: bool,
) -> Result<Value, String> {
    let base = format!("/api/repository/{space}/branch/main");
    if journal.exists() {
        if synchronize {
            request_method(router, &format!("{base}/sync/status"), "", "GET")
                .await
                .map_err(|_| "Retained state authorization failed.".to_owned())?;
        }
        if recovering {
            request(router, &format!("{base}/sync/push"), "")
                .await
                .map_err(|_| "Retained state publication could not be confirmed.".to_owned())?;
            clear_journal(journal).map_err(|_| "Could not clear recovery journal.".to_owned())?;
            return Ok(json!({ "published_retained_state": true }));
        }
        if writing {
            return Err("An earlier evaluation is unresolved. Query retained state and explicitly recover it before writing again.".into());
        }
    } else {
        if recovering {
            return Err("No retained evaluation to recover.".into());
        }
        if synchronize {
            request(router, &format!("{base}/sync/pull"), "")
                .await
                .map_err(|_| "Space pull failed; no evaluation was dispatched.".to_owned())?;
        }
    }
    if writing && synchronize {
        create_private_dir(journal.parent().expect("journal has a parent"))
            .and_then(|()| create_journal(journal))
            .map_err(|_| {
                "Could not persist evaluation journal; no evaluation was dispatched.".to_owned()
            })?;
    }
    let mut result = request(router, &format!("{base}/evaluate?transact={writing}"), document)
        .await
        .map_err(|_| {
            if writing {
                "Evaluation could not be confirmed. Do not repeat it; query the space first."
                    .to_owned()
            } else {
                "The worker query could not be completed.".to_owned()
            }
        })?;
    if writing && synchronize {
        request(router, &format!("{base}/sync/push"), "")
            .await
            .map_err(|_| {
                "Applied locally; remote push could not be confirmed. Do not repeat the write."
                    .to_owned()
            })?;
        clear_journal(journal).map_err(|_| {
            "Publication succeeded but recovery journal could not be cleared.".to_owned()
        })?;
    } else if journal.exists() {
        result["recovery"] = json!({"required": true, "message": "Retained local state; publication or evaluation outcome is unresolved. Do not repeat the write."});
    }
    Ok(result)
}

/// One request through the ordinary router, keeping the status and any JSON
/// body so refusals can be classified.
async fn send(
    router: &mut Router,
    method: &str,
    path: &str,
    content_type: &str,
    body: String,
) -> Result<(bool, Value), String> {
    let failed = || "The local peer could not complete the request.".to_owned();
    let response = router
        .call(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", content_type)
                .body(Body::from(body))
                .map_err(|_| failed())?,
        )
        .await
        .map_err(|_| failed())?;
    let ok = response.status().is_success();
    let bytes = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .map_err(|_| failed())?;
    Ok((ok, serde_json::from_slice(&bytes).unwrap_or(Value::Null)))
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

/// A private root with one exclusive lock. Unlike the agent-link adapter
/// there are no replica or byte caps: this is one person's own peer.
fn lock_root(root: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    create_private_dir(root)?;
    let meta = std::fs::symlink_metadata(root)?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        return Err(std::io::Error::other("Private root required"));
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.join("peer.lock"))?;
    // Block rather than fail: two hosts on one machine (desktop chat and a
    // coding agent) may share this peer and take turns.
    lock.lock().map_err(std::io::Error::other)?;
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("tonk-peer-spike-{}", rand::random::<u64>()))
    }

    #[tokio::test]
    async fn peer_keeps_one_identity_and_starts_with_no_spaces() {
        let root = scratch();
        let remote: url::Url = "http://127.0.0.1:9/ucan/".parse().unwrap();
        let mut peer = LocalPeerWorker::new(root.clone(), remote.clone());
        let first = peer.call("tonk_list_spaces", &json!({})).await.unwrap();
        assert!(first["peer"].as_str().unwrap().starts_with("did:key:"));
        assert_eq!(first["spaces"], json!([]));
        let mut again = LocalPeerWorker::new(root.clone(), remote);
        let second = again.call("tonk_list_spaces", &json!({})).await.unwrap();
        assert_eq!(first["peer"], second["peer"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn peer_refuses_links_for_another_service_and_unjoined_spaces() {
        let root = scratch();
        let remote: url::Url = "https://tonk.foundation/ucan/".parse().unwrap();
        let mut peer = LocalPeerWorker::new(root.clone(), remote);
        let seed = bs58::encode([7u8; 32]).into_string();
        let foreign = format!(
            "https://evil.example/space/did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK#{seed}"
        );
        let refused = peer
            .call("tonk_join", &json!({ "link": foreign }))
            .await
            .unwrap_err();
        assert!(refused.contains("different Tonk service"), "{refused}");
        assert!(!refused.contains(&seed));
        let unjoined = peer
            .call(
                "tonk_query",
                &json!({
                    "space": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                    "document": "concept:\n",
                }),
            )
            .await
            .unwrap_err();
        assert!(unjoined.contains("has not joined"), "{unjoined}");
        assert!(peer.call("tonk_join", &json!({})).await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
