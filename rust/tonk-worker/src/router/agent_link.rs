//! Native agent-link adapter, not a public HTTP route. Each invocation supplies
//! a complete expanded agent link. The host pins the trusted access-service URL.
use std::path::{Path, PathBuf};

use axum::{Router, body::Body, http::Request};
use dialog_capability::{Subject, did};
use dialog_credentials::{Credential, Ed25519Signer, SignerCredential};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::time::Timestamp;
use serde_json::Value;
use tonk_invite::connection::{AgentInvite, candidate_build_scopes, grant_set_id};
use tower_service::Service;

use crate::{TonkState, worker::DefaultSpace};

/// Per-invocation scoped authority over the ordinary worker.
pub struct AgentLinkWorker {
    root: PathBuf,
    remote: url::Url,
}

impl AgentLinkWorker {
    /// Pin a host-owned data directory and independently trusted remote.
    pub fn new(root: PathBuf, remote: url::Url) -> Self {
        Self { root, remote }
    }

    /// Validate on every call, pull before exposing data, evaluate once and push
    /// writes once. Errors deliberately exclude upstream details and credentials.
    pub async fn call(&mut self, tool: &str, link: &str, document: &str) -> Result<Value, String> {
        self.execute(tool, link, document, true).await
    }

    async fn execute(
        &mut self,
        tool: &str,
        link: &str,
        document: &str,
        synchronize: bool,
    ) -> Result<Value, String> {
        let recovering = tool == "publish_retained";
        let writing = match tool {
            "tonk_query" | "publish_retained" => false,
            "tonk_evaluate" => true,
            _ => return Err("Tool unavailable.".into()),
        };
        if !recovering && (document.trim().is_empty() || document.len() > 32_000) {
            return Err("Provide one inline notation document, up to 32 KB.".into());
        }
        if link.len() > 160_000 {
            return Err("Agent link exceeds limit.".into());
        }
        let invalid = || "Invalid, expired or untrusted agent link.".to_owned();
        let hint = AgentInvite::inspect_url(link, Timestamp::now())
            .await
            .map_err(|_| invalid())?;
        let invite = AgentInvite::parse_url(
            link,
            &candidate_build_scopes(&hint.subject),
            &self.remote,
            Timestamp::now(),
        )
        .await
        .map_err(|_| invalid())?;
        let grants = invite.grants();
        let subject = grants.subject().to_string();
        let cids = grants
            .chains()
            .iter()
            .map(|chain| {
                chain
                    .proof_cids()
                    .last()
                    .expect("validated chain")
                    .to_string()
            })
            .collect::<Vec<_>>();
        let id = grant_set_id(&subject, grants.recipient().as_ref(), &cids);
        let _lock = lock_root(&self.root)
            .map_err(|_| "Agent storage is unavailable or busy.".to_owned())?;
        let directory = self.root.join(id);
        if !directory.exists()
            && std::fs::read_dir(&self.root)
                .map_err(|_| "Could not inspect agent storage.".to_owned())?
                .count()
                >= 65
        {
            return Err("Agent storage capacity reached.".into());
        }
        let journal = directory.join("pending-evaluation");
        let state = open(&directory, &invite)
            .await
            .map_err(|_| "Could not open the isolated agent worker.".to_owned())?;
        let (mut router, state, hub) = crate::api_router_with_state(state);
        let base = format!("/api/repository/{subject}/branch/main");
        let result = async {
            if journal.exists() {
                // Fresh authorization without pulling over unpublished local state.
                if synchronize {
                    request_method(&mut router, &format!("{base}/sync/status"), "", "GET")
                        .await.map_err(|_| "Retained state authorization failed.".to_owned())?;
                }
                if recovering {
                    request(&mut router, &format!("{base}/sync/push"), "").await
                        .map_err(|_| "Retained state publication could not be confirmed.".to_owned())?;
                    clear_journal(&journal).map_err(|_| "Could not clear recovery journal.".to_owned())?;
                    return Ok(serde_json::json!({"published_retained_state": true}));
                }
                if writing {
                    return Err("An earlier evaluation is unresolved. Query retained state and explicitly recover it before writing again.".into());
                }
            } else {
                if recovering { return Err("No retained evaluation to recover.".into()); }
                if synchronize {
                    request(&mut router, &format!("{base}/sync/pull"), "").await
                        .map_err(|_| "Space pull failed; no evaluation was dispatched.".to_owned())?;
                }
            }
            if writing && synchronize {
                create_journal(&journal).map_err(|_| "Could not persist evaluation journal; no evaluation was dispatched.".to_owned())?;
            }
            let mut result = request(&mut router, &format!("{base}/evaluate?transact={writing}"), document)
                .await.map_err(|_| if writing {
                    "Evaluation could not be confirmed. Do not repeat it; query the space first.".to_owned()
                } else { "The worker query could not be completed.".to_owned() })?;
            if writing && synchronize {
                request(&mut router, &format!("{base}/sync/push"), "").await
                    .map_err(|_| "Applied locally; remote push could not be confirmed. Do not repeat the write.".to_owned())?;
                clear_journal(&journal).map_err(|_| "Publication succeeded but recovery journal could not be cleared.".to_owned())?;
            } else if journal.exists() {
                result["recovery"] = serde_json::json!({"required": true, "message": "Retained local state; publication or evaluation outcome is unresolved. Do not repeat the write."});
            }
            Ok(result)
        }.await;
        state.read().await.shutdown();
        hub.shutdown().await;
        result
    }
}

async fn request(router: &mut Router, path: &str, document: &str) -> Result<Value, ()> {
    request_method(router, path, document, "POST").await
}

async fn request_method(
    router: &mut Router,
    path: &str,
    document: &str,
    method: &str,
) -> Result<Value, ()> {
    let response = router
        .call(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "text/plain")
                .body(Body::from(document.to_owned()))
                .map_err(|_| ())?,
        )
        .await
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let body = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .map_err(|_| ())?;
    serde_json::from_slice(&body).map_err(|_| ())
}

fn lock_root(root: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(root)?;
    fn validate(path: &Path, bytes: &mut u64, files: &mut usize) -> std::io::Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink()
            || (!meta.is_dir() && (!meta.is_file() || meta.nlink() != 1))
        {
            return Err(std::io::Error::other("Private regular storage required"));
        }
        if meta.is_dir() {
            for entry in std::fs::read_dir(path)? {
                validate(&entry?.path(), bytes, files)?;
            }
        } else {
            *bytes += meta.len();
            *files += 1;
        }
        if *bytes > 192 * 1024 * 1024 || *files > 3500 {
            return Err(std::io::Error::other("Storage capacity reached"));
        }
        Ok(())
    }
    let root_meta = std::fs::symlink_metadata(root)?;
    if !root_meta.is_dir() || root_meta.permissions().mode() & 0o077 != 0 {
        return Err(std::io::Error::other("Private root required"));
    }
    let lock_path = root.join("worker.lock");
    if lock_path.exists() {
        let meta = std::fs::symlink_metadata(&lock_path)?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err(std::io::Error::other("Invalid lock"));
        }
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(lock_path)?;
    lock.try_lock().map_err(std::io::Error::other)?;
    validate(root, &mut 0, &mut 0)?;
    Ok(lock)
}

fn create_journal(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.sync_all()?;
    std::fs::File::open(path.parent().unwrap())?.sync_all()
}

fn clear_journal(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)?;
    std::fs::File::open(path.parent().unwrap())?.sync_all()
}

async fn open(
    directory: &Path,
    invite: &AgentInvite,
) -> Result<TonkState, Box<dyn std::error::Error>> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let directory = directory.canonicalize()?;
    let location = Directory::At(directory.to_string_lossy().into_owned());
    let (credentials, _) =
        tonk_account::peer::open_system::<DefaultSpace>(location.clone()).await?;
    let key = tonk_account::peer::kept_key_path(&directory, "hosted");
    if !key.exists() {
        let signer = Ed25519Signer::import(invite.secret_seed()).await?;
        Subject::from(did!("local:storage"))
            .attenuate(storage_fx::Storage)
            .attenuate(Location::new(location, "hosted"))
            .create(Credential::Signer(SignerCredential::from(signer)))
            .perform(&credentials)
            .await?;
    }
    let state = TonkState::open_native(&directory).await?;
    if state.profile.did() != *invite.grants().recipient() {
        state.shutdown();
        return Err("agent identity mismatch".into());
    }
    for chain in invite.grants().chains() {
        state
            .profile
            .access()
            .save(UcanDelegation(chain.clone()))
            .perform(&state.operator)
            .await?;
    }
    super::join::mount_replica(
        &state,
        invite.grants().subject(),
        Some(invite.grants().remote().as_str()),
        None,
    )
    .await?;
    super::repository::record_initialized_replica_in_profile(&state, invite.grants().subject())
        .await?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Signer;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal;

    async fn link(seed: [u8; 32], owner: &Signer, remote: &url::Url) -> String {
        let recipient = Ed25519Signer::import(&seed).await.unwrap();
        let now = Timestamp::now();
        let deadline = Timestamp::try_from((now.to_unix() + 3600) as i128).unwrap();
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
                .meta(tonk_invite::home_address_meta(remote))
                .try_build()
                .await
                .unwrap();
            chains.push(DelegationChain::new(grant));
        }
        AgentInvite::new(seed, chains, &scopes, remote, now)
            .await
            .unwrap()
            .to_url("https://tonk.example/agent")
            .unwrap()
    }

    #[tokio::test]
    async fn agent_link_evaluates_and_queries_the_ordinary_worker() {
        let root = std::env::temp_dir().join(format!("tonk-agent-spike-{}", rand::random::<u64>()));
        let remote = "http://127.0.0.1:9/ucan/".parse().unwrap();
        let owner = Signer::from(Ed25519Signer::generate().await.unwrap());
        let agent_link = link([42; 32], &owner, &remote).await;
        let mut spike = AgentLinkWorker::new(root.clone(), remote.clone());
        // Only this private test path bypasses sync. Exercise real cryptographic
        // import, native storage and the unchanged worker evaluate routes.
        let document = "concept!: &spike-record\n  with:\n    title: { the: example.spike/title, as: text, description: Test title }\nspike-record!:\n  this: urn:spike:record\n  title: Agent link works\n";
        let written = spike
            .execute("tonk_evaluate", &agent_link, document, false)
            .await
            .unwrap();
        assert_ne!(written["revision_before"], written["revision_after"]);
        let query = spike
            .execute("tonk_query", &agent_link, "spike-record:\n", false)
            .await
            .unwrap();
        assert!(query.to_string().contains("Agent link works"));
        assert_eq!(query["revision_before"], query["revision_after"]);
        let dry = spike
            .execute(
                "tonk_query",
                &agent_link,
                "spike-record!:\n  this: urn:spike:record\n  title: Must not persist\n",
                false,
            )
            .await
            .unwrap();
        assert_eq!(dry["revision_before"], dry["revision_after"]);
        let query = spike
            .execute("tonk_query", &agent_link, "spike-record:\n", false)
            .await
            .unwrap();
        assert!(!query.to_string().contains("Must not persist"));
        assert!(!query.to_string().contains("tonk-agent-v2"));

        assert!(
            spike
                .execute("tonk_query", "", "spike-record:\n", false)
                .await
                .is_err()
        );
        let mut wrong = url::Url::parse(&agent_link).unwrap();
        wrong.set_fragment(Some("tonk-agent-v2=invalid"));
        assert!(
            spike
                .execute("tonk_query", wrong.as_str(), "spike-record:\n", false)
                .await
                .is_err()
        );
        let other = link([43; 32], &owner, &remote).await;
        let isolated = spike
            .execute("tonk_query", &other, "concept:\n", false)
            .await
            .unwrap();
        assert!(!isolated.to_string().contains("spike-record"));
        // The public adapter never serves cached content when its remote cannot
        // authorize the pull. This is NOT a live revocation or remote sync test.
        let denied = spike
            .call("tonk_query", &agent_link, "spike-record:\n")
            .await
            .unwrap_err();
        assert_eq!(denied, "Space pull failed; no evaluation was dispatched.");
        std::fs::remove_dir_all(root).unwrap();
    }
}
