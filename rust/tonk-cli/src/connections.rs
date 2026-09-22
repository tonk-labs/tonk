//! Isolated, resumable import of ordinary invitation authority.
//!
//! The outer registered directory contains public binding/journal metadata and a
//! private native credential store. Repository data is nested so legacy clients
//! cannot load it with their ambient profile. No account ceremony runs here.

use crate::peer::NativePeer;
use anyhow::{Context, Result, ensure};
use dialog_capability::{Subject, did};
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier, SignerCredential};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt as _};
use dialog_reactor::Reactor;
use dialog_repository::{RepositoryExt as _, SiteAddress};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, time::Timestamp};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use tonk_invite::connection::{AgentInvite, SpaceGrantBundle, candidate_build_scopes};
use url::Url;

pub use tonk_invite::connection::InvitationHint;

/// Public marker on the outer connection directory.
pub const MARKER_FILE: &str = "connection.json";
/// Marker preventing generic opens of nested replica data.
pub const DATA_MARKER_FILE: &str = ".connection-data";
const PROFILE_NAME: &str = "invitation";
const DATA_DIRECTORY: &str = "data";
const CREDENTIAL_DIRECTORY: &str = "credentials";

/// Immutable public authority selector, duplicated in the space registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionBinding {
    /// Binding format version, currently one.
    pub version: u32,
    /// BLAKE3 of the subject, recipient and sorted original grant CIDs.
    pub id: String,
    /// The single remote space subject.
    pub subject: String,
    /// The retained invitation public identity.
    pub recipient: String,
    /// Original leaf delegation CIDs, sorted for deterministic resume.
    pub grant_cids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Credentials,
    Mounted,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_request: Option<String>,
    binding: ConnectionBinding,
    remote: String,
    // Public signed proof chains. The invitation seed only lives in the native
    // credential store and is never duplicated in this journal.
    grants: Vec<String>,
    validated_at: u64,
    phase: Phase,
}

/// Fully checked invitation, retained in memory only until credential import.
/// Debug delegates to AgentInvite's redacted representation.
#[derive(Debug)]
pub struct ValidatedConnection {
    invite: AgentInvite,
    binding: ConnectionBinding,
    validated_at: u64,
    directory: Option<PathBuf>,
    terminal_request: Option<String>,
}

impl ValidatedConnection {
    /// Retain the original requested final directory before credential checkpoints.
    pub fn with_directory(mut self, directory: &Path) -> Result<Self> {
        self.directory = Some(directory.canonicalize()?);
        Ok(self)
    }

    /// Immutable public binding for choosing deterministic replica storage.
    pub fn binding(&self) -> &ConnectionBinding {
        &self.binding
    }
}

/// Verify the bearer before exposing claimed service routing for discovery.
pub async fn inspect_link(link: &str) -> Result<InvitationHint> {
    AgentInvite::inspect_url(link, Timestamp::now()).await
}

/// Validate all grants and key correspondence after trusted route discovery.
pub async fn validate_link(link: &str, trusted_remote: &Url) -> Result<ValidatedConnection> {
    let now = Timestamp::now();
    let hint = AgentInvite::inspect_url(link, now).await?;
    let invite = AgentInvite::parse_url(
        link,
        &candidate_build_scopes(&hint.subject),
        trusted_remote,
        now,
    )
    .await?;
    let binding = derive_binding(invite.grants());
    Ok(ValidatedConnection {
        invite,
        binding,
        validated_at: now.to_unix(),
        directory: None,
        terminal_request: None,
    })
}

pub(crate) fn derive_binding(grants: &SpaceGrantBundle) -> ConnectionBinding {
    let subject = grants.subject().to_string();
    let recipient = grants.recipient().to_string();
    let mut grant_cids: Vec<_> = grants
        .chains()
        .iter()
        .map(|chain| {
            chain
                .proof_cids()
                .last()
                .expect("validated chain")
                .to_string()
        })
        .collect();
    grant_cids.sort();
    ConnectionBinding {
        version: 1,
        id: tonk_invite::connection::grant_set_id(&subject, &recipient, &grant_cids),
        subject,
        recipient,
        grant_cids,
    }
}

fn read_manifest(root: &Path) -> Result<Manifest> {
    let file = root.join(MARKER_FILE);
    let metadata = std::fs::symlink_metadata(&file).context("connection binding is missing")?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= 2 * 1024 * 1024,
        "connection binding is not a bounded regular file"
    );
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(file)?).context("connection binding is malformed")?;
    ensure!(
        manifest.binding.version == 1
            && manifest.binding.id.len() == 64
            && manifest
                .binding
                .id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "unsupported connection binding"
    );
    Ok(manifest)
}

/// Public delivery source for truthful display; never selects signing authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionSource {
    /// Ordinary browser-generated bearer import.
    Invitation,
    /// Grants addressed to a retained CLI key for this exact request.
    Terminal(String),
}
/// Inspect validated-format source metadata without changing credentials.
pub fn source_at(root: &Path) -> Result<Option<ConnectionSource>> {
    if binding_at(root)?.is_none() {
        return Ok(None);
    }
    let manifest = read_manifest(root)?;
    Ok(Some(match manifest.terminal_request {
        Some(id) => {
            ensure!(
                id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
                "malformed terminal request metadata"
            );
            ConnectionSource::Terminal(id)
        }
        None => ConnectionSource::Invitation,
    }))
}

/// Read a scoped marker. Malformed markers fail; they are never legacy fallback.
pub fn binding_at(root: &Path) -> Result<Option<ConnectionBinding>> {
    match std::fs::symlink_metadata(root.join(MARKER_FILE)) {
        Ok(_) => Ok(Some(read_manifest(root)?.binding)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ensure!(
                !root.join(DATA_MARKER_FILE).exists()
                    && !root.join(DATA_DIRECTORY).join(DATA_MARKER_FILE).exists(),
                "nested connection data requires its outer binding"
            );
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Refuse generic account-profile loading of marked connection roots or data.
pub(crate) fn reject_generic_open(root: &Path) -> Result<()> {
    ensure!(
        binding_at(root)?.is_none(),
        "this space uses isolated connection authority; open its registered binding"
    );
    Ok(())
}

// Test-feature-only process termination immediately after a durable checkpoint.
// Release binaries do not read this environment variable.
fn checkpoint(_phase: &str) {
    #[cfg(feature = "integration-tests")]
    if std::env::var("TONK_TEST_CONNECTION_CHECKPOINT")
        .ok()
        .as_deref()
        == Some(_phase)
    {
        std::process::exit(86);
    }
}

pub(crate) fn reject_unsafe_tree(root: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        !metadata.file_type().is_symlink(),
        "connection storage contains a symlink"
    );
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "connection storage contains a special file"
    );
    if metadata.is_dir() {
        for entry in std::fs::read_dir(root)? {
            reject_unsafe_tree(&entry?.path())?;
        }
    }
    Ok(())
}

pub(crate) fn private_directory(root: &Path) -> Result<()> {
    if root.exists() {
        let metadata = std::fs::symlink_metadata(root)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "connection directory must not be a symlink"
        );
    } else {
        let mut missing = Vec::new();
        let mut ancestor = root;
        while !ancestor.exists() {
            missing.push(ancestor.to_path_buf());
            let Some(parent) = ancestor.parent() else {
                break;
            };
            ancestor = parent;
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
        // Flush newly created directory entries too, not only their contents.
        for directory in missing {
            File::open(&directory)?.sync_all()?;
            if let Some(parent) = directory.parent() {
                File::open(parent)?.sync_all()?;
            }
        }
    }
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn import_lock(root: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join(".connection.lock"))?;
    file.lock().context("failed to lock connection import")?;
    Ok(file)
}

pub(crate) fn atomic_public(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(root.join(name)).map_err(|error| error.error)?;
    File::open(root)?.sync_all()?;
    Ok(())
}

fn save_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    atomic_public(root, MARKER_FILE, &serde_json::to_vec_pretty(manifest)?)
}

// The upstream native provider serializes credentials but does not promise a
// durability barrier. Harden and flush every initial credential/grant file before
// exposing the imported binding to any remote operation.
fn sync_private_tree(root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "connection storage contains a symlink"
    );
    if metadata.is_dir() {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
        for entry in std::fs::read_dir(root)? {
            sync_private_tree(&entry?.path())?;
        }
    } else {
        ensure!(
            metadata.is_file(),
            "connection storage contains a special file"
        );
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o600))?;
    }
    File::open(root)?.sync_all()?;
    Ok(())
}

fn profile_directory(root: &Path) -> Directory {
    Directory::At(
        root.join(CREDENTIAL_DIRECTORY)
            .to_string_lossy()
            .into_owned(),
    )
}

async fn load_profile(
    root: &Path,
    storage: &Storage<NativeSpace>,
    binding: &ConnectionBinding,
) -> Result<NativePeer> {
    let profile = dialog_peer::Peer::new()
        .storage(storage.clone())
        .load(dialog_effects::storage::Location::new(
            profile_directory(root),
            PROFILE_NAME,
        ))
        .await
        .context("connection credential is missing or corrupt; no account fallback is permitted")?;
    ensure!(
        profile.did().to_string() == binding.recipient,
        "connection credential does not match its binding"
    );
    Ok(profile)
}

async fn validate_manifest(manifest: &Manifest) -> Result<SpaceGrantBundle> {
    let subject: Did = manifest
        .binding
        .subject
        .parse()
        .context("connection subject is malformed")?;
    let recipient: Did = manifest
        .binding
        .recipient
        .parse()
        .context("connection recipient is malformed")?;
    let remote = Url::parse(&manifest.remote).context("connection remote is malformed")?;
    let chains = manifest
        .grants
        .iter()
        .map(|encoded| {
            let bytes = hex::decode(encoded).context("connection proof encoding is malformed")?;
            DelegationChain::try_from(bytes.as_slice()).context("connection proof is malformed")
        })
        .collect::<Result<Vec<_>>>()?;
    // Recheck signatures and exact persisted authority at import time so expired
    // local replicas stay readable. Every remote invocation uses the current clock.
    let imported_at = Timestamp::try_from(manifest.validated_at as i128)?;
    let grants = SpaceGrantBundle::validate(
        chains,
        &recipient,
        &candidate_build_scopes(&subject),
        &remote,
        imported_at,
    )
    .await?;
    ensure!(
        derive_binding(&grants) == manifest.binding,
        "connection grant set does not match its binding"
    );
    Ok(grants)
}

/// Durably import identity/grants and mount a main-only replica, without network.
/// Repeating the same import resumes its checkpoints; differing state is refused.
pub async fn import_at(
    root: &Path,
    connection: &ValidatedConnection,
    store: crate::space::SpaceStore,
) -> Result<ConnectionBinding> {
    reject_unsafe_tree(root)?;
    ensure!(
        !root.join(crate::site::REPO_NAME).exists(),
        "connection outer directory contains contradictory legacy data"
    );
    if root.exists() {
        match binding_at(root)? {
            Some(binding) => ensure!(
                binding == connection.binding,
                "connection directory belongs to different authority"
            ),
            None => ensure!(
                std::fs::read_dir(root)?
                    .all(|entry| entry.is_ok_and(|entry| entry.file_name() == ".connection.lock")),
                "connection directory contains unrelated files"
            ),
        }
    }
    private_directory(root)?;
    let root = root.canonicalize()?;
    let _lock = import_lock(&root)?;
    reject_unsafe_tree(&root)?;
    let mut manifest = match binding_at(&root)? {
        Some(binding) => {
            ensure!(
                binding == connection.binding,
                "connection directory belongs to different authority"
            );
            let manifest = read_manifest(&root)?;
            ensure!(
                manifest.terminal_request == connection.terminal_request,
                "connection delivery source changed"
            );
            validate_manifest(&manifest).await?;
            manifest
        }
        None => {
            ensure!(
                std::fs::read_dir(&root)?
                    .all(|entry| entry.is_ok_and(|entry| entry.file_name() == ".connection.lock")),
                "connection directory contains unrelated files"
            );
            let manifest = Manifest {
                terminal_request: connection.terminal_request.clone(),
                binding: connection.binding.clone(),
                remote: connection.invite.grants().remote().to_string(),
                grants: connection
                    .invite
                    .grants()
                    .chains()
                    .iter()
                    .map(|chain| chain.to_bytes().map(hex::encode))
                    .collect::<std::result::Result<_, _>>()?,
                validated_at: connection.validated_at,
                phase: Phase::Preparing,
            };
            save_manifest(&root, &manifest)?;
            manifest
        }
    };
    if let Some(directory) = &connection.directory {
        if let Some(previous) =
            crate::handoff::pending_scoped_directory(&root, &manifest.binding.id)?
        {
            ensure!(
                &previous == directory,
                "connection has a different pending directory; resume its original binding"
            );
        } else {
            crate::handoff::remember_scoped_directory(&root, &manifest.binding.id, directory)?;
        }
    }
    private_directory(&root.join(CREDENTIAL_DIRECTORY))?;
    private_directory(&root.join(DATA_DIRECTORY))?;
    let data_marker = root.join(DATA_DIRECTORY).join(DATA_MARKER_FILE);
    if data_marker.exists() {
        ensure!(
            std::fs::read_to_string(&data_marker)? == manifest.binding.id,
            "connection data marker mismatch"
        );
    } else {
        ensure!(
            manifest.phase == Phase::Preparing,
            "connection data marker is missing"
        );
        atomic_public(
            &root.join(DATA_DIRECTORY),
            DATA_MARKER_FILE,
            manifest.binding.id.as_bytes(),
        )?;
    }
    let storage = Storage::<NativeSpace>::default();
    let key_path = root
        .join(CREDENTIAL_DIRECTORY)
        .join(PROFILE_NAME)
        .join("credential/key/self");
    if !key_path.exists() {
        ensure!(
            manifest.phase == Phase::Preparing,
            "connection credential is missing; refusing to regenerate it"
        );
        let signer = Ed25519Signer::import(connection.invite.secret_seed()).await?;
        let credential = Credential::Signer(SignerCredential::from(signer));
        Subject::from(did!("local:storage"))
            .attenuate(storage_fx::Storage)
            .attenuate(Location::new(profile_directory(&root), PROFILE_NAME))
            .create(credential)
            .perform(&storage)
            .await
            .context("failed to persist invitation identity")?;
    }
    let profile = load_profile(&root, &storage, &manifest.binding).await?;
    sync_private_tree(&root.join(CREDENTIAL_DIRECTORY))?;
    if manifest.phase == Phase::Preparing {
        manifest.phase = Phase::Credentials;
        save_manifest(&root, &manifest)?;
        checkpoint("credentials");
    }
    let site = assemble(
        &root,
        &manifest,
        profile,
        storage,
        store,
        manifest.phase != Phase::Ready,
    )
    .await?;
    sync_private_tree(&root.join(CREDENTIAL_DIRECTORY))?;
    sync_private_tree(&root.join(DATA_DIRECTORY))?;
    manifest.phase = Phase::Ready;
    save_manifest(&root, &manifest)?;
    drop(site);
    Ok(manifest.binding)
}

/// Reopen the retained connection, recovering an interrupted local mount if needed.
pub async fn open_bound(
    root: &Path,
    binding: &ConnectionBinding,
    store: crate::space::SpaceStore,
) -> Result<crate::site::TonkSite> {
    reject_unsafe_tree(root)?;
    ensure!(
        !root.join(crate::site::REPO_NAME).exists(),
        "connection outer directory contains contradictory legacy data"
    );
    let preflight = read_manifest(root)?;
    ensure!(
        &preflight.binding == binding,
        "registry and connection binding disagree"
    );
    let root = root.canonicalize()?;
    let _lock = import_lock(&root)?;
    let mut manifest = read_manifest(&root)?;
    ensure!(
        &manifest.binding == binding,
        "registry and connection binding disagree"
    );
    validate_manifest(&manifest).await?;
    ensure!(
        std::fs::read_to_string(root.join(DATA_DIRECTORY).join(DATA_MARKER_FILE))? == binding.id,
        "connection data marker does not match its binding"
    );
    let storage = Storage::<NativeSpace>::default();
    let profile = load_profile(&root, &storage, binding).await?;
    let incomplete = manifest.phase != Phase::Ready;
    let site = assemble(&root, &manifest, profile, storage, store, incomplete).await?;
    if incomplete {
        sync_private_tree(&root.join(CREDENTIAL_DIRECTORY))?;
        sync_private_tree(&root.join(DATA_DIRECTORY))?;
        manifest.phase = Phase::Ready;
        save_manifest(&root, &manifest)?;
    }
    Ok(site)
}

async fn assemble(
    root: &Path,
    manifest: &Manifest,
    profile: NativePeer,
    storage: Storage<NativeSpace>,
    store: crate::space::SpaceStore,
    initialize: bool,
) -> Result<crate::site::TonkSite> {
    let data = root.join(DATA_DIRECTORY);
    let expires = Timestamp::try_from((Timestamp::now().to_unix() + 3600) as i128)?;
    let peer =
        crate::peer::peer_for(&profile, Directory::At(data.to_string_lossy().into_owned())).await?;
    let operator = peer
        .derive(b"tonk-scoped-connection")
        .await?
        .allow(peer.access().claim(Subject::any()).expires(expires))
        .build()
        .await?;
    let grants = validate_manifest(manifest).await?;
    if initialize {
        for chain in grants.chains() {
            profile
                .access()
                .save(UcanDelegation(chain.clone()))
                .perform(&operator)
                .await?;
        }
        // The subject is verifier-only; no ownership or account prefix is minted.
        if !data.join("main/credential/key/self").exists() {
            let verifier: Ed25519Verifier = manifest
                .binding
                .subject
                .parse()
                .map_err(|error| anyhow::anyhow!("invalid connection subject: {error:?}"))?;
            Subject::from(profile.did())
                .attenuate(Space::new(crate::site::REPO_NAME))
                .create(Credential::from(verifier))
                .perform(&operator)
                .await?;
        }
    }
    let repository = profile
        .space(crate::site::REPO_NAME)
        .load()
        .perform(&operator)
        .await?;
    ensure!(
        matches!(repository.credential(), Credential::Verifier(_)),
        "connection replica unexpectedly contains owner signing authority"
    );
    ensure!(
        repository.did().to_string() == manifest.binding.subject,
        "connection replica subject mismatch"
    );
    if initialize && manifest.phase != Phase::Ready {
        sync_private_tree(&root.join(CREDENTIAL_DIRECTORY))?;
        sync_private_tree(&root.join(DATA_DIRECTORY))?;
        let mut mounted = manifest.clone();
        mounted.phase = Phase::Mounted;
        save_manifest(root, &mounted)?;
        checkpoint("mounted");
    }
    let wrapper = crate::account_authority::wrap_scoped(
        operator,
        profile.clone(),
        store.clone(),
        grants.chains().to_vec(),
    );
    let site = crate::site::TonkSite {
        root: data,
        profile: profile.clone(),
        operator: wrapper,
        repository,
        reactor: Reactor::new(profile.credential().clone()),
        account_store: store,
    };
    if initialize {
        match site
            .repository
            .remote("origin")
            .load()
            .perform(&site.operator)
            .await
        {
            Ok(_) => {}
            Err(dialog_repository::LoadRemoteError::NotFound { .. }) => {
                site.repository
                    .remote("origin")
                    .create(SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
                        manifest.remote.clone(),
                    )))
                    .perform(&site.operator)
                    .await?;
            }
            Err(error) => return Err(error.into()),
        }
        let remote = site
            .repository
            .remote("origin")
            .load()
            .perform(&site.operator)
            .await?;
        let upstream = remote.branch("main").open().perform(&site.operator).await?;
        let branch = site.branch().await?;
        if branch.handle().upstreams().is_empty() {
            branch
                .handle()
                .set_upstream(&upstream)
                .perform(&site.operator)
                .await?;
        }
    }
    let remote = site
        .repository
        .remote("origin")
        .load()
        .perform(&site.operator)
        .await?;
    ensure!(
        remote.address().site()
            == &SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
                manifest.remote.clone()
            )),
        "connection remote does not match its trusted binding"
    );
    let main_upstreams = site.branch().await?.handle().upstreams();
    ensure!(main_upstreams.iter().count() == 1 && main_upstreams.iter().all(|upstream| matches!(upstream,
        dialog_repository::Upstream::Remote { remote, branch, .. } if remote == "origin" && branch == "main")),
        "connection main branch must track exactly its trusted origin/main");
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await?;
    ensure!(
        meta.upstreams().is_empty(),
        "connection cannot track the management branch"
    );
    Ok(site)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_varsig::Principal as _;

    #[dialog_common::test]
    async fn connection_retained_expired_grants_allow_offline_edits_but_no_remote_fallback()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("expired");
        let store = crate::space::SpaceStore::at(temp.path().join("ambient"));
        let owner = Signer::from(Ed25519Signer::generate().await?);
        let seed = [90; 32];
        let recipient = Ed25519Signer::import(&seed).await?;
        let now = Timestamp::now().to_unix();
        let imported_at = Timestamp::try_from((now - 120) as i128)?;
        let expired = Timestamp::try_from((now - 60) as i128)?;
        let remote = Url::parse("http://127.0.0.1:9/ucan/")?;
        let scopes = candidate_build_scopes(&owner.did());
        let mut chains = Vec::new();
        for scope in &scopes {
            let grant = DelegationBuilder::new()
                .issuer(owner.clone())
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .expiration(expired)
                .meta(tonk_invite::home_address_meta(&remote))
                .try_build()
                .await?;
            chains.push(DelegationChain::new(grant));
        }
        // Model a credential validated and installed at its historical instant.
        // The public current-time importer must refuse this same expired link.
        let invite = AgentInvite::new(seed, chains, &scopes, &remote, imported_at).await?;
        let link = invite.to_url("https://tonk.network/connect")?;
        assert!(validate_link(&link, &remote).await.is_err());
        let binding = derive_binding(invite.grants());
        let retained = ValidatedConnection {
            invite,
            binding: binding.clone(),
            validated_at: imported_at.to_unix(),
            directory: None,
            terminal_request: None,
        };
        import_at(&root, &retained, store.clone()).await?;
        let site = open_bound(&root, &binding, store.clone()).await?;
        crate::eval::run_against_site(&site, crate::eval::Source::Inline("attribute!: &offline-title\n  description: \"Offline title\"\n  the: xyz.test/offline-title\n  as: text\n  cardinality: one\n".into()), crate::eval::Options::default()).await?;
        let tree = site.branch().await?.handle().revision().unwrap().tree;
        let denied = crate::sync::push(&site).await.unwrap_err();
        assert!(
            matches!(&denied, crate::sync::SyncError::Rejected { .. }),
            "{denied:?}"
        );
        assert!(!denied.to_string().contains("account login"));
        drop(site);
        let reopened = open_bound(&root, &binding, store.clone()).await?;
        assert_eq!(
            reopened.branch().await?.handle().revision().unwrap().tree,
            tree
        );
        assert!(!store.root().exists());
        Ok(())
    }
}
