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

/// A long-lived local peer over one private data directory.
///
/// Only the process holding the directory lock (see [`try_lock`]) may open
/// it; that owner keeps the worker open for its whole life so it can sync in
/// the background, and other processes forward their calls to it.
pub struct LocalPeer {
    router: Router,
    state: crate::AppState,
    hub: std::sync::Arc<crate::router::lsp::LspHub>,
    root: PathBuf,
    remote: url::Url,
    did: String,
}

impl LocalPeer {
    /// Open the peer's one persistent profile, generating its key on first
    /// use. The caller must hold the lock returned by [`try_lock`].
    pub async fn open(root: PathBuf, remote: url::Url) -> Result<Self, String> {
        create_private_dir(&root).map_err(|_| "Peer storage is unavailable.".to_owned())?;
        let state = TonkState::open_native(&root.join("peer"))
            .await
            .map_err(|_| "Could not open the local peer.".to_owned())?;
        let did = state.profile.did().to_string();
        let (router, state, hub) = crate::api_router_with_state(state);
        Ok(Self {
            router,
            state,
            hub,
            root,
            remote,
            did,
        })
    }

    /// This peer's own DID: the device identity that joins spaces.
    pub fn did(&self) -> &str {
        &self.did
    }

    /// Run one tool. Arguments are tool-specific; errors exclude upstream
    /// details and never echo a link.
    pub async fn call(&mut self, tool: &str, arguments: &Value) -> Result<Value, String> {
        self.execute(tool, arguments, true).await
    }

    /// One background sync pass over every space with pending changes.
    pub async fn sync(&mut self) -> Result<(), String> {
        let (ok, _) = send(
            &mut self.router,
            "POST",
            "/api/sync",
            "text/plain",
            String::new(),
        )
        .await?;
        if ok {
            Ok(())
        } else {
            Err("Background sync did not complete.".into())
        }
    }

    /// Release query streams and the worker.
    pub async fn close(self) {
        self.state.read().await.shutdown();
        self.hub.shutdown().await;
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
                let link = self.expand(link).await?;
                if is_agent_link(&link) {
                    return self.join_agent(&link).await;
                }
                self.pin(&link).await?;
                join(&mut self.router, &link).await
            }
            "tonk_list_spaces" => list(&mut self.router, self.did.clone()).await,
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
                if !list(&mut self.router, self.did.clone()).await?["spaces"]
                    .as_array()
                    .is_some_and(|spaces| spaces.iter().any(|entry| entry["subject"] == space))
                {
                    return Err(
                        "This peer has not joined that space. Ask for a share link and call tonk_join."
                            .to_owned(),
                    );
                }
                let journal = self.root.join("pending").join(space.replace(':', "_"));
                evaluate(
                    &mut self.router,
                    space,
                    document,
                    &journal,
                    writing,
                    recovering,
                    synchronize,
                )
                .await
            }
            _ => Err("Tool unavailable.".into()),
        }
    }

    /// Expand a `/@/` shortcut into the link it stands for. Only the pinned
    /// service is asked, and the private fragment is never sent: the server
    /// answers a fragment-free request and the fragment is reattached.
    async fn expand(&self, link: &str) -> Result<String, String> {
        let parsed = url::Url::parse(link).map_err(|_| "That is not a Tonk share link.".to_owned())?;
        if !parsed.path().starts_with("/@/") {
            return Ok(link.to_owned());
        }
        if parsed.origin() != self.remote.origin() {
            return Err(self.foreign());
        }
        super::join::resolve_shortcut(link)
            .await
            .map_err(|_| "Could not expand the shortcut link.".to_owned())
    }

    /// Redeem an agent link into this peer's own identity.
    ///
    /// An agent link grants a fixed set of scopes to the key in its fragment.
    /// The peer uses that key once, to delegate each grant onward to itself
    /// with the same command, policy, expiry and signed route, then keeps
    /// only the extended chains. Access lasts as long as the link's grants;
    /// the link's key is never stored.
    async fn join_agent(&mut self, link: &str) -> Result<Value, String> {
        use dialog_credentials::{Ed25519Signer, Signer};
        use dialog_ucan::UcanDelegation;
        use dialog_ucan_core::{DelegationBuilder, time::Timestamp};
        use tonk_invite::connection::{AgentInvite, SpaceGrantBundle, candidate_build_scopes};
        use tonk_schema::prelude::DidExt;

        let invalid = || "Invalid, expired or untrusted agent link.".to_owned();
        let now = Timestamp::now();
        let hint = AgentInvite::inspect_url(link, now).await.map_err(|_| invalid())?;
        let scopes = candidate_build_scopes(&hint.subject);
        let invite = AgentInvite::parse_url(link, &scopes, &self.remote, now)
            .await
            .map_err(|_| self.foreign())?;
        let link_key = Signer::from(
            Ed25519Signer::import(invite.secret_seed())
                .await
                .map_err(|_| invalid())?,
        );
        let subject = invite.grants().subject().clone();
        let renewed = list(&mut self.router, self.did.clone()).await?["spaces"]
            .as_array()
            .is_some_and(|spaces| spaces.iter().any(|entry| entry["subject"] == subject.to_string()));
        let state = self.state.write().await;
        let peer = state.profile.did();
        let mut chains = Vec::new();
        for chain in invite.grants().chains() {
            let leaf = chain.proofs().last().ok_or_else(invalid)?;
            let hop = DelegationBuilder::new()
                .issuer(link_key.clone())
                .audience(&peer)
                .subject(leaf.subject().clone())
                .command(leaf.command().0.clone())
                .policy(leaf.policy().clone())
                .expiration(leaf.expiration().ok_or_else(invalid)?)
                .meta(leaf.meta().clone())
                .try_build()
                .await
                .map_err(|_| invalid())?;
            chains.push(chain.push(hop).map_err(|_| invalid())?);
        }
        let grants = SpaceGrantBundle::validate(chains, &peer, &scopes, &self.remote, now)
            .await
            .map_err(|_| invalid())?;
        let stored = async {
            for chain in grants.chains() {
                state
                    .profile
                    .access()
                    .save(UcanDelegation(chain.clone()))
                    .perform(&state.operator)
                    .await
                    .map_err(|_| ())?;
            }
            if !renewed {
                super::join::mount_replica(
                    &state,
                    grants.subject(),
                    Some(grants.remote().as_str()),
                    None,
                )
                .await
                .map_err(|_| ())?;
                super::repository::record_initialized_replica_in_profile(&state, grants.subject())
                    .await
                    .map_err(|_| ())?;
            }
            Ok::<(), ()>(())
        }
        .await;
        drop(state);
        stored.map_err(|()| "Could not save the space membership.".to_owned())?;
        let name = request_method(
            &mut self.router,
            &format!("/api/repository/{}", subject.repo_key()),
            "",
            "GET",
        )
        .await
        .ok()
        .and_then(|info| info["label"].as_str().map(str::to_owned));
        Ok(json!({ "subject": subject.to_string(), "name": name, "renewed": renewed }))
    }

    fn foreign(&self) -> String {
        format!(
            "This link belongs to a different Tonk service than this peer trusts ({}).",
            self.remote.origin().ascii_serialization()
        )
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
            _ => Err(self.foreign()),
        }
    }
}

/// Agent links carry their key in a versioned fragment.
fn is_agent_link(link: &str) -> bool {
    url::Url::parse(link)
        .ok()
        .and_then(|url| url.fragment().map(|fragment| fragment.starts_with("tonk-agent-")))
        .unwrap_or(false)
}

async fn join(router: &mut Router, link: &str) -> Result<Value, String> {
    let body = json!({ "url": link }).to_string();
    let (ok, value) = send(
        router,
        "POST",
        "/api/profile/join",
        "application/json",
        body,
    )
    .await?;
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
    let mut result = request(
        router,
        &format!("{base}/evaluate?transact={writing}"),
        document,
    )
    .await
    .map_err(|_| {
        if writing {
            "Evaluation could not be confirmed. Do not repeat it; query the space first.".to_owned()
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

/// Try to become the owner of a peer directory. `Ok(None)` means another
/// process owns it; the lock is held for as long as the returned file lives.
/// There are no replica or byte caps: this is one person's own peer.
pub fn try_lock(root: &Path) -> std::io::Result<Option<std::fs::File>> {
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
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("tonk-agent-peer-spike-{}", rand::random::<u64>()))
    }

    #[tokio::test]
    async fn one_owner_keeps_one_identity_and_starts_with_no_spaces() {
        let root = scratch();
        let remote: url::Url = "http://127.0.0.1:9/ucan/".parse().unwrap();
        let lock = try_lock(&root)
            .unwrap()
            .expect("first process owns the peer");
        assert!(
            try_lock(&root).unwrap().is_none(),
            "a second owner is refused"
        );
        let mut peer = LocalPeer::open(root.clone(), remote.clone()).await.unwrap();
        let first = peer.call("tonk_list_spaces", &json!({})).await.unwrap();
        assert!(first["peer"].as_str().unwrap().starts_with("did:key:"));
        assert_eq!(first["peer"], peer.did());
        assert_eq!(first["spaces"], json!([]));
        peer.close().await;
        drop(lock);
        let _lock = try_lock(&root).unwrap().expect("ownership passes on");
        let mut again = LocalPeer::open(root.clone(), remote).await.unwrap();
        let second = again.call("tonk_list_spaces", &json!({})).await.unwrap();
        assert_eq!(first["peer"], second["peer"]);
        again.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn peer_refuses_links_for_another_service_and_unjoined_spaces() {
        let root = scratch();
        let remote: url::Url = "https://tonk.foundation/ucan/".parse().unwrap();
        let _lock = try_lock(&root).unwrap().unwrap();
        let mut peer = LocalPeer::open(root.clone(), remote).await.unwrap();
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
        peer.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
