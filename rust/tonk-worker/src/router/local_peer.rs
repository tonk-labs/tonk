//! Native local peer, not a public HTTP route. Unlike the agent-link adapter,
//! the peer keeps its own long-lived identity: one profile, whose key it
//! generates on first use and never receives from a link. Pasted share links
//! are redeemed through the ordinary join, which delegates the membership to
//! this peer's account, so the link itself is no longer needed afterwards and
//! the space owner can withdraw it. Like the browser peer, it joins spaces on
//! any Tonk service the user's link names.
use std::path::{Path, PathBuf};

use axum::{Router, body::Body, http::Request};
use serde_json::{Value, json};
use tower_service::Service;

use super::agent_link::{clear_journal, create_journal, request_detailed};
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
    did: String,
}

impl LocalPeer {
    /// Open the peer's one persistent profile, generating its key on first
    /// use. The caller must hold the lock returned by [`try_lock`].
    pub async fn open(root: PathBuf) -> Result<Self, String> {
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
            did,
        })
    }

    /// The peer's ordinary worker router, for serving it to local views. It
    /// shares this peer's state; requests through it act as this peer.
    pub fn router(&self) -> Router {
        self.router.clone()
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

    /// One sync pass: push every space with pending changes, then pull every
    /// open one. `POST /api/sync` only acknowledges natively (the browser's
    /// service worker is what drains on it), so call the drain directly.
    pub async fn sync(&self) {
        crate::router::drain_sync(&self.state).await;
    }

    /// The worker state, for syncing without holding this peer: a drain
    /// waits on the network, and tool calls should not wait behind it.
    pub fn sync_state(&self) -> crate::AppState {
        self.state.clone()
    }

    /// Stop serving without consuming the peer: end the worker's generation
    /// and release its live query streams, for when another process takes
    /// over the directory while handles to this one are still alive.
    pub async fn retire(&self) {
        self.state.read().await.shutdown();
        self.hub.shutdown().await;
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

    /// Expand a `/@/` shortcut into the link it stands for. The private
    /// fragment is never sent: the server answers a fragment-free request and
    /// the fragment is reattached.
    async fn expand(&self, link: &str) -> Result<String, String> {
        let parsed =
            url::Url::parse(link).map_err(|_| "That is not a Tonk share link.".to_owned())?;
        if !parsed.path().starts_with("/@/") {
            return Ok(link.to_owned());
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
        let hint = AgentInvite::inspect_url(link, now)
            .await
            .map_err(|_| invalid())?;
        let scopes = candidate_build_scopes(&hint.subject);
        // The link's own signed route is the service it is redeemed against.
        let remote = hint.remote;
        let invite = AgentInvite::parse_url(link, &scopes, &remote, now)
            .await
            .map_err(|_| invalid())?;
        let link_key = Signer::from(
            Ed25519Signer::import(invite.secret_seed())
                .await
                .map_err(|_| invalid())?,
        );
        let subject = invite.grants().subject().clone();
        let renewed = list(&mut self.router, self.did.clone()).await?["spaces"]
            .as_array()
            .is_some_and(|spaces| {
                spaces
                    .iter()
                    .any(|entry| entry["subject"] == subject.to_string())
            });
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
        let grants = SpaceGrantBundle::validate(chains, &peer, &scopes, &remote, now)
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
        let name = request_detailed(
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
}

/// Agent links carry their key in a versioned fragment.
fn is_agent_link(link: &str) -> bool {
    url::Url::parse(link)
        .ok()
        .and_then(|url| {
            url.fragment()
                .map(|fragment| fragment.starts_with("tonk-agent-"))
        })
        .unwrap_or(false)
}

async fn join(router: &mut Router, link: &str) -> Result<Value, String> {
    let body = join_body(link);
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

/// The join request the peer sends: the link, and the peer declaring
/// itself an agent, so the space's roster records it as one rather than
/// as a person.
fn join_body(link: &str) -> String {
    json!({ "url": link, "agent": true }).to_string()
}

async fn list(router: &mut Router, peer: String) -> Result<Value, String> {
    let failed = || "Could not read this peer's spaces.".to_owned();
    let profile = request_detailed(router, "/api/profile", "", "GET")
        .await
        .map_err(|_| failed())?;
    let mut spaces = Vec::new();
    for entry in profile["space"].as_array().into_iter().flatten() {
        let key = entry["key"].as_str().unwrap_or_default();
        let label = request_detailed(router, &format!("/api/repository/{key}"), "", "GET")
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
            request_detailed(router, &format!("{base}/sync/status"), "", "GET")
                .await
                .map_err(|error| format!("Retained state authorization failed. ({error})"))?;
        }
        if recovering {
            request_detailed(router, &format!("{base}/sync/push"), "", "POST")
                .await
                .map_err(|error| {
                    format!("Retained state publication could not be confirmed. ({error})")
                })?;
            clear_journal(journal)
                .map_err(|error| format!("Could not clear recovery journal. ({error})"))?;
            return Ok(json!({ "published_retained_state": true }));
        }
        if writing {
            // Recover on the agent's behalf rather than latching: publish
            // whatever the unresolved write left locally, so the space is
            // current, then refuse this write once. Writing blindly now could
            // repeat the earlier one; the agent queries and decides.
            request_detailed(router, &format!("{base}/sync/push"), "", "POST")
                .await
                .map_err(|error| {
                    format!(
                        "An earlier write's outcome is unresolved and its local state could not be published yet; nothing new was written. Try again shortly. ({error})"
                    )
                })?;
            clear_journal(journal)
                .map_err(|error| format!("Could not clear recovery journal. ({error})"))?;
            return Err("An earlier write's outcome was unresolved. Its local state is now published, so the space is current, and nothing new was written. Query the space to see whether that write applied, then write again only if it did not.".into());
        }
    } else {
        if recovering {
            return Err("No retained evaluation to recover.".into());
        }
        if synchronize {
            request_detailed(router, &format!("{base}/sync/pull"), "", "POST")
                .await
                .map_err(|error| {
                    format!("Space pull failed; no evaluation was dispatched. ({error})")
                })?;
        }
    }
    if writing && synchronize {
        create_private_dir(journal.parent().expect("journal has a parent"))
            .and_then(|()| create_journal(journal))
            .map_err(|_| {
                "Could not persist evaluation journal; no evaluation was dispatched.".to_owned()
            })?;
    }
    let mut result = request_detailed(
        router,
        &format!("{base}/evaluate?transact={writing}"),
        document,
        "POST",
    )
    .await
    .map_err(|error| {
        if !writing {
            format!("The worker query could not be completed. ({error})")
        } else if refused(&error) {
            // The worker turned the document down (analysis, validation)
            // before applying anything: there is nothing to recover.
            let _ = clear_journal(journal);
            format!("Nothing was written: the worker refused the document. Fix it and write again.\n{error}")
        } else {
            format!("Evaluation could not be confirmed. Do not repeat it; query the space first. ({error})")
        }
    })?;
    if writing && synchronize {
        request_detailed(router, &format!("{base}/sync/push"), "", "POST")
            .await
            .map_err(|error| {
                format!(
                    "Applied locally; remote push could not be confirmed. Do not repeat the write. ({error})"
                )
            })?;
        clear_journal(journal).map_err(|_| {
            "Publication succeeded but recovery journal could not be cleared.".to_owned()
        })?;
    } else if journal.exists() {
        result["recovery"] = json!({"required": true, "message": "An earlier write's outcome is unresolved; its local state is not yet published. The next write publishes it first, then asks you to check before writing again. Do not repeat the write blindly."});
    }
    Ok(result)
}

/// Whether a worker failure is a refusal of the request itself (a 4xx),
/// as opposed to an outcome that is unknown (transport failure, 5xx).
fn refused(error: &str) -> bool {
    error.starts_with('4') && error.as_bytes().get(3) == Some(&b' ')
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
        std::env::temp_dir().join(format!("tonk-peer-spike-{}", rand::random::<u64>()))
    }

    #[tokio::test]
    async fn one_owner_keeps_one_identity_and_starts_with_no_spaces() {
        let root = scratch();
        let lock = try_lock(&root)
            .unwrap()
            .expect("first process owns the peer");
        assert!(
            try_lock(&root).unwrap().is_none(),
            "a second owner is refused"
        );
        let mut peer = LocalPeer::open(root.clone()).await.unwrap();
        let first = peer.call("tonk_list_spaces", &json!({})).await.unwrap();
        assert!(first["peer"].as_str().unwrap().starts_with("did:key:"));
        assert_eq!(first["peer"], peer.did());
        assert_eq!(first["spaces"], json!([]));
        peer.close().await;
        drop(lock);
        let _lock = try_lock(&root).unwrap().expect("ownership passes on");
        let mut again = LocalPeer::open(root.clone()).await.unwrap();
        let second = again.call("tonk_list_spaces", &json!({})).await.unwrap();
        assert_eq!(first["peer"], second["peer"]);
        again.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn peer_refuses_malformed_links_and_unjoined_spaces() {
        let root = scratch();
        let _lock = try_lock(&root).unwrap().unwrap();
        let mut peer = LocalPeer::open(root.clone()).await.unwrap();
        let seed = bs58::encode([7u8; 32]).into_string();
        let malformed = format!("not a link #{seed}");
        let refused = peer
            .call("tonk_join", &json!({ "link": malformed }))
            .await
            .unwrap_err();
        assert!(!refused.contains(&seed), "{refused}");
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

    #[test]
    fn it_declares_itself_an_agent_when_it_joins() {
        let body: Value = serde_json::from_str(&join_body("https://example.test/i#seed")).unwrap();
        assert_eq!(body["url"], "https://example.test/i#seed");
        assert_eq!(body["agent"], true);
    }

    #[test]
    fn only_client_errors_count_as_refusals() {
        assert!(refused("400 Bad Request: analyze failed"));
        assert!(refused("409 Conflict: stale"));
        assert!(!refused("500 Internal Server Error: boom"));
        assert!(!refused("connection reset"));
        assert!(!refused("4000 bytes"));
    }

    #[test]
    fn worker_errors_read_as_the_analyzer_reports_them() {
        let body = br#"{"error":{"kind":"analyze","message":"`count` expects Integer, got Text","code":"E_TYPE","range":{"start":{"line":3,"character":7}}}}"#;
        assert_eq!(
            super::super::agent_link::worker_error(body),
            "E_TYPE: `count` expects Integer, got Text (line 4, column 8)"
        );
        assert_eq!(super::super::agent_link::worker_error(b" plain "), "plain");
    }

    // An unresolved write never latches: the next write tries to publish
    // its retained state, and keeps the marker only while that fails.
    #[tokio::test]
    async fn unresolved_writes_are_published_before_refusing_once() {
        let root = scratch();
        let _lock = try_lock(&root).unwrap().unwrap();
        let mut peer = LocalPeer::open(root.clone()).await.unwrap();
        let space = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let journal = root.join("pending").join(space.replace(':', "_"));
        create_private_dir(journal.parent().unwrap()).unwrap();
        create_journal(&journal).unwrap();
        // The space is unknown here, so publishing fails: still unresolved.
        let held = evaluate(&mut peer.router, space, "x", &journal, true, false, false)
            .await
            .unwrap_err();
        assert!(held.contains("could not be published yet"), "{held}");
        assert!(journal.exists(), "the marker stays while publishing fails");
        peer.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
