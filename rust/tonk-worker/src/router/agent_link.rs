//! Native agent-link adapter, not a public HTTP route. Each invocation supplies
//! a complete expanded agent link. The host pins the trusted access-service URL.
use std::path::{Path, PathBuf};

use axum::{Router, body::Body, http::Request};
use dialog_capability::{Subject, did};
use dialog_credentials::{Credential, Ed25519Signer, SignerCredential};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Principal as _;
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
        if writing {
            // Syntax rejection has no effects. Parse the whole document before
            // opening a replica or journaling an evaluation as uncertain.
            let parsed = tonk_notation::parse(document);
            if let Some(first) = parsed.diagnostics.first() {
                return Err(format!(
                    "Invalid notation (line {}, column {}); no evaluation was dispatched.",
                    first.range.start.line + 1,
                    first.range.start.character + 1
                ));
            }
            if parsed.syntax.is_none() {
                return Err("Invalid notation; no evaluation was dispatched.".into());
            }
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
        let state = open(&directory, &invite).await.map_err(|error| match error
            .downcast_ref::<ReplicaLocationError>()
        {
            Some(reason) => format!("Could not open the isolated agent worker ({reason})."),
            None => "Could not open the isolated agent worker.".to_owned(),
        })?;
        let (mut router, state, hub) = crate::api_router_with_state(state);
        let base = format!("/api/repository/{subject}/branch/main");
        let result = async {
            if journal.exists() {
                // Fresh authorization without pulling over unpublished local state.
                if synchronize {
                    request_method(&mut router, &format!("{base}/sync/status"), "", "GET")
                        .await.map_err(|error| error.describe("Retained state authorization failed."))?;
                }
                if recovering {
                    request(&mut router, &format!("{base}/sync/push"), "").await
                        .map_err(|error| error.describe("Retained state publication could not be confirmed."))?;
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
                        .map_err(|error| error.describe(PULL_FAILED))?;
                }
            }
            if writing && synchronize {
                create_journal(&journal).map_err(|_| "Could not persist evaluation journal; no evaluation was dispatched.".to_owned())?;
            }
            let mut result = request(&mut router, &format!("{base}/evaluate?transact={writing}"), document)
                .await.map_err(|error| error.describe(if writing {
                    "Evaluation could not be confirmed. Do not repeat it; query the space first."
                } else { "The worker query could not be completed." }))?;
            if writing && synchronize {
                request(&mut router, &format!("{base}/sync/push"), "").await
                    .map_err(|error| error.describe("Applied locally; remote push could not be confirmed. Do not repeat the write."))?;
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

/// Only stable categories cross the agent boundary. Never retain an upstream
/// message/body: either can contain credentials or submitted notation.
#[derive(Debug, PartialEq, Eq)]
enum RequestFailure {
    Unavailable,
    Http {
        status: u16,
        code: Option<&'static str>,
    },
}

const PULL_FAILED: &str = "Space pull failed; no evaluation was dispatched.";

impl RequestFailure {
    /// Append only the status and allowlisted code to a fixed message.
    fn describe(&self, message: &str) -> String {
        match self {
            Self::Unavailable => message.to_owned(),
            Self::Http { status, code } => match code {
                Some(code) => format!("{message} (HTTP {status}; {code})"),
                None => format!("{message} (HTTP {status})"),
            },
        }
    }
}

fn sync_error_code(body: &[u8]) -> Option<&'static str> {
    let value: Value = serde_json::from_slice(body).ok()?;
    match value.get("error")?.get("code")?.as_str()? {
        "CREDENTIAL_REVOKED" => Some("CREDENTIAL_REVOKED"),
        "NOT_PROVISIONED" => Some("NOT_PROVISIONED"),
        "SYNC_CONFLICT" => Some("SYNC_CONFLICT"),
        "SYNC_UNAVAILABLE" => Some("SYNC_UNAVAILABLE"),
        "UPSTREAM_ERROR" => Some("UPSTREAM_ERROR"),
        _ => None,
    }
}

async fn request(router: &mut Router, path: &str, document: &str) -> Result<Value, RequestFailure> {
    request_method(router, path, document, "POST").await
}

async fn request_method(
    router: &mut Router,
    path: &str,
    document: &str,
    method: &str,
) -> Result<Value, RequestFailure> {
    let response = router
        .call(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "text/plain")
                .body(Body::from(document.to_owned()))
                .map_err(|_| RequestFailure::Unavailable)?,
        )
        .await
        .map_err(|_| RequestFailure::Unavailable)?;
    let status = response.status();
    if !status.is_success() {
        // Error diagnostics need only a small JSON envelope. Oversized or
        // malformed errors still retain status, never any response text.
        let body = axum::body::to_bytes(response.into_body(), 16_384).await;
        return Err(RequestFailure::Http {
            status: status.as_u16(),
            code: body.ok().and_then(|body| sync_error_code(&body)),
        });
    }
    let body = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .map_err(|_| RequestFailure::Unavailable)?;
    serde_json::from_slice(&body).map_err(|_| RequestFailure::Unavailable)
}

/// Like [`request_method`], but says why it failed: the worker's status and
/// the start of its body. Only for callers whose user owns the peer; the
/// hosted connection keeps worker errors opaque.
pub(super) async fn request_detailed(
    router: &mut Router,
    path: &str,
    document: &str,
    method: &str,
) -> Result<Value, String> {
    let response = router
        .call(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "text/plain")
                .body(Body::from(document.to_owned()))
                .map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 2_000_000)
        .await
        .map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("{status}: {}", worker_error(&body)));
    }
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

/// A worker error body as an agent can act on it: the analyzer's message
/// with its code and position when the body is the worker's error shape,
/// else the start of the body.
pub(super) fn worker_error(body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(body)
        && let Some(message) = value["error"]["message"].as_str()
    {
        let code = value["error"]["code"]
            .as_str()
            .or(value["error"]["kind"].as_str());
        let start = &value["error"]["range"]["start"];
        let at = match (start["line"].as_u64(), start["character"].as_u64()) {
            // Ranges are zero-based; editors and people count from one.
            (Some(line), Some(column)) => format!(" (line {}, column {})", line + 1, column + 1),
            _ => String::new(),
        };
        return match code {
            Some(code) => format!("{code}: {}{at}", clip(message, 4000)),
            None => format!("{}{at}", clip(message, 4000)),
        };
    }
    clip(String::from_utf8_lossy(body).trim(), 1000)
}

fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
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

pub(super) fn create_journal(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.sync_all()?;
    std::fs::File::open(path.parent().unwrap())?.sync_all()
}

pub(super) fn clear_journal(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)?;
    std::fs::File::open(path.parent().unwrap())?.sync_all()
}

/// A checkpoint may move the host root, but not the grant partition or the
/// subject's storage name. This never follows the old absolute path.
fn same_scoped_replica(previous: &Location, current: &Location, subject: &str) -> bool {
    let (Directory::At(previous_root), Directory::At(current_root)) =
        (&previous.directory, &current.directory)
    else {
        return false;
    };
    let previous_root = Path::new(previous_root);
    let current_root = Path::new(current_root);
    previous.name == subject
        && current.name == subject
        && previous_root.is_absolute()
        && !previous_root.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        && previous_root.file_name() == Some(std::ffi::OsStr::new("spaces"))
        && current_root.file_name() == Some(std::ffi::OsStr::new("spaces"))
        && previous_root.parent().and_then(Path::file_name)
            == current_root.parent().and_then(Path::file_name)
}

/// A fixed relocation refusal reason. It names no path, DID or upstream text,
/// so it may be reported to the agent for operator diagnosis.
#[derive(Debug)]
struct ReplicaLocationError(&'static str);

impl std::fmt::Display for ReplicaLocationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for ReplicaLocationError {}

fn refuse(reason: &'static str) -> Box<dyn std::error::Error> {
    Box::new(ReplicaLocationError(reason))
}

/// Repair only the authorized replica's profile-local location after restore.
/// Verify the restored credential and data first: missing data must not be
/// replaced by a fresh empty replica, particularly when a pending evaluation is
/// retained. Records in a layout this repair does not own are left unchanged.
async fn relocate_replica(
    state: &TonkState,
    subject: &dialog_varsig::Did,
) -> Result<(), Box<dyn std::error::Error>> {
    let key = subject.to_string();
    let peer = state.profile.did();
    let recorded = dialog_repository::spaces::find(
        state.profile.state(),
        &peer,
        subject.as_ref(),
        &state.profile,
    )
    .await
    .map_err(|_| refuse("replica registry unavailable"))?;
    let (recorded_subject, previous) = match recorded.as_slice() {
        [] => return Ok(()),
        [entry] => entry,
        _ => return Err(refuse("ambiguous scoped replica location")),
    };
    if recorded_subject != subject {
        return Err(refuse("scoped replica identity mismatch"));
    }
    let current = state.registry.space_location(&key);
    if previous == &current || !same_scoped_replica(previous, &current, &key) {
        return Ok(());
    }
    let credential = Subject::from(did!("local:storage"))
        .attenuate(storage_fx::Storage)
        .attenuate(current.clone())
        .load()
        .perform(state.profile.storage())
        .await
        .map_err(|_| refuse("restored replica missing"))?;
    if credential.did() != *subject {
        return Err(refuse("restored replica identity mismatch"));
    }
    // Every mounted replica has replica metadata; its absence means the
    // restore carried the credential without the replica's branch data.
    let restored = dialog_repository::Repository::from(credential)
        .branch(super::repository::META_BRANCH)
        .open()
        .perform(&state.operator)
        .await
        .map_err(|_| refuse("restored replica data missing"))?;
    if restored.revision().is_none() {
        return Err(refuse("restored replica data missing"));
    }
    dialog_repository::spaces::record(
        state.profile.state(),
        &peer,
        subject,
        &key,
        &current,
        &state.profile,
    )
    .await
    .map_err(|_| refuse("replica registry unavailable"))?;
    Ok(())
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
    if let Err(error) = relocate_replica(&state, invite.grants().subject()).await {
        state.shutdown();
        return Err(error);
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

    #[test]
    fn relocation_accepts_only_the_same_grant_and_subject() {
        let current = Location::new(Directory::At("/new/grant-id/spaces".into()), "subject");
        let old = Location::new(Directory::At("/old/grant-id/spaces".into()), "subject");
        assert!(same_scoped_replica(&old, &current, "subject"));
        for previous in [
            Location::new(Directory::At("/old/other-grant/spaces".into()), "subject"),
            Location::new(Directory::At("/old/grant-id/other".into()), "subject"),
            Location::new(
                Directory::At("/old/grant-id/spaces".into()),
                "other-subject",
            ),
            Location::new(Directory::At("/old/../grant-id/spaces".into()), "subject"),
        ] {
            assert!(!same_scoped_replica(&previous, &current, "subject"));
        }
    }

    #[tokio::test]
    async fn relocation_preserves_retained_evaluation_and_branch_revision() {
        let root =
            std::env::temp_dir().join(format!("tonk-agent-relocate-{}", rand::random::<u64>()));
        let original = root.join("original");
        let restored = root.join("restored");
        let remote = "http://127.0.0.1:9/ucan/".parse().unwrap();
        let owner = Signer::from(Ed25519Signer::generate().await.unwrap());
        let agent_link = link([44; 32], &owner, &remote).await;
        let mut worker = AgentLinkWorker::new(original.clone(), remote.clone());
        let written = worker.execute("tonk_evaluate", &agent_link,
            "concept!: &retained\n  with:\n    title: { the: example.retained/title, as: text, description: Test title }\nretained!:\n  this: urn:retained\n  title: Keep this work\n", false).await.unwrap();
        let grant = std::fs::read_dir(&original)
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.path().is_dir())
            .unwrap()
            .file_name();
        let journal = Path::new(&grant).join("pending-evaluation");
        std::fs::write(original.join(&journal), b"retained-marker").unwrap();
        std::fs::rename(&original, &restored).unwrap();
        assert!(!original.exists());
        // An old path may still exist in a test or overlapping rollout. Give
        // it a fresh empty replica: the restored worker must use its own data,
        // not succeed accidentally by following the obsolete location.
        let mut replacement = AgentLinkWorker::new(original.clone(), remote.clone());
        replacement
            .execute("tonk_query", &agent_link, "concept:", false)
            .await
            .unwrap();
        assert!(original.exists());
        let mut worker = AgentLinkWorker::new(restored.clone(), remote);
        let read = worker
            .execute("tonk_query", &agent_link, "retained:", false)
            .await
            .unwrap();
        assert_eq!(read["revision_after"], written["revision_after"]);
        assert!(read.to_string().contains("Keep this work"));
        assert_eq!(read["recovery"]["required"], true);
        assert_eq!(
            std::fs::read(restored.join(&journal)).unwrap(),
            b"retained-marker"
        );
        assert!(
            worker
                .execute(
                    "tonk_evaluate",
                    &agent_link,
                    "retained!:\n  this: urn:retained\n  title: Must not replace\n",
                    false
                )
                .await
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn relocation_refuses_missing_or_wrong_restored_credential() {
        let root = std::env::temp_dir().join(format!(
            "tonk-agent-reject-relocate-{}",
            rand::random::<u64>()
        ));
        let directory = root.join("grant-id");
        let state = TonkState::open_native(&directory).await.unwrap();
        let subject = Ed25519Signer::generate().await.unwrap().did();
        let wrong = Ed25519Signer::generate().await.unwrap().did();
        let key = subject.to_string();
        let old = Location::new(Directory::At("/old/grant-id/spaces".into()), key.clone());
        dialog_repository::spaces::record(
            state.profile.state(),
            &state.profile.did(),
            &subject,
            &key,
            &old,
            &state.profile,
        )
        .await
        .unwrap();
        let current = state.registry.space_location(&key);
        assert!(relocate_replica(&state, &subject).await.is_err());
        assert!(!directory.join("spaces").join(&key).exists());
        tonk_account::peer::mount_verifier(state.profile.storage(), current, &wrong)
            .await
            .unwrap();
        assert_eq!(
            relocate_replica(&state, &subject)
                .await
                .unwrap_err()
                .to_string(),
            "restored replica identity mismatch"
        );
        let recorded = dialog_repository::spaces::find(
            state.profile.state(),
            &state.profile.did(),
            &key,
            &state.profile,
        )
        .await
        .unwrap();
        assert_eq!(recorded, vec![(subject, old)]);
        state.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn relocation_refuses_credential_without_replica_data_and_keeps_legacy_records() {
        let root = std::env::temp_dir().join(format!(
            "tonk-agent-partial-relocate-{}",
            rand::random::<u64>()
        ));
        let directory = root.join("grant-id");
        let state = TonkState::open_native(&directory).await.unwrap();
        let peer = state.profile.did();
        let subject = Ed25519Signer::generate().await.unwrap().did();
        let key = subject.to_string();
        let old = Location::new(Directory::At("/old/grant-id/spaces".into()), key.clone());
        dialog_repository::spaces::record(
            state.profile.state(),
            &peer,
            &subject,
            &key,
            &old,
            &state.profile,
        )
        .await
        .unwrap();
        let current = state.registry.space_location(&key);
        tonk_account::peer::mount_verifier(state.profile.storage(), current, &subject)
            .await
            .unwrap();
        assert_eq!(
            relocate_replica(&state, &subject)
                .await
                .unwrap_err()
                .to_string(),
            "restored replica data missing"
        );

        // A layout this repair does not own is left for the ordinary mount.
        let legacy = Location::new(Directory::Current, key.clone());
        dialog_repository::spaces::record(
            state.profile.state(),
            &peer,
            &subject,
            &key,
            &legacy,
            &state.profile,
        )
        .await
        .unwrap();
        relocate_replica(&state, &subject).await.unwrap();
        let recorded =
            dialog_repository::spaces::find(state.profile.state(), &peer, &key, &state.profile)
                .await
                .unwrap();
        assert_eq!(recorded, vec![(subject, legacy)]);
        state.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }

    async fn error_response(status: u16, body: String) -> RequestFailure {
        let mut router = Router::new().route(
            "/fixture",
            axum::routing::post(move || async move {
                (axum::http::StatusCode::from_u16(status).unwrap(), body)
            }),
        );
        request(&mut router, "/fixture", "").await.unwrap_err()
    }

    #[tokio::test]
    async fn pull_diagnostics_preserve_only_status_and_allowlisted_code() {
        for code in [
            "CREDENTIAL_REVOKED",
            "NOT_PROVISIONED",
            "SYNC_CONFLICT",
            "SYNC_UNAVAILABLE",
            "UPSTREAM_ERROR",
        ] {
            let body = serde_json::json!({"error": {
                "code": code, "kind": "upstream", "message": "private-secret",
                "agent_link": "https://example.test/#tonk-agent-v2=private-secret"
            }})
            .to_string();
            let failure = error_response(503, body).await;
            assert_eq!(
                failure,
                RequestFailure::Http {
                    status: 503,
                    code: Some(code)
                }
            );
            assert_eq!(
                failure.describe(PULL_FAILED),
                format!("Space pull failed; no evaluation was dispatched. (HTTP 503; {code})")
            );
        }
    }

    #[tokio::test]
    async fn pull_diagnostics_do_not_echo_unknown_malformed_or_oversized_errors() {
        for body in [
            serde_json::json!({"error": {"code": "private-secret", "message": "private-secret"}})
                .to_string(),
            "private-secret".to_owned(),
            format!(
                "{{\"error\":{{\"code\":\"SYNC_UNAVAILABLE\",\"message\":\"{}\"}}}}",
                "private-secret".repeat(2000)
            ),
        ] {
            let failure = error_response(502, body).await;
            assert_eq!(
                failure,
                RequestFailure::Http {
                    status: 502,
                    code: None
                }
            );
            assert_eq!(
                failure.describe(PULL_FAILED),
                "Space pull failed; no evaluation was dispatched. (HTTP 502)"
            );
        }
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

        // The hosted failure followed these read-only documents. Exercise the
        // same sequence against a closed/reopened native replica each time;
        // malformed queries may fail, but must not poison a later valid read.
        let revision = query["revision_after"].clone();
        for document in [
            "concept:\n  name: ?",
            "help:",
            "concept:\n  name:",
            "concept:\n  this: concept:note",
        ] {
            let _ = spike
                .execute("tonk_query", &agent_link, document, false)
                .await;
        }
        let after_errors = spike
            .execute("tonk_query", &agent_link, "spike-record:\n", false)
            .await
            .unwrap();
        assert_eq!(after_errors["revision_after"], revision);
        assert!(after_errors.to_string().contains("Agent link works"));

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
        assert!(denied.starts_with("Space pull failed; no evaluation was dispatched."));
        assert!(!denied.contains("tonk-agent-v2"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
