//! Durable CLI-owned request keys and atomic selected-space publication.
//!
//! The account registry is never used as signing authority or an approval hint.
//! An interrupted initial attempt installs nothing until its entire authenticated
//! selection has been prepared. Cancellation is local, not remote revocation.

use crate::{
    connections::{self, ConnectionBinding, ValidatedConnection},
    space::{SpaceEntry, SpaceStore},
};
use anyhow::{Context, Result, ensure};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use tonk_invite::terminal::{Approval, LinkRequest, REQUEST_TTL_SECONDS};
use url::Url;

mod command;
mod deliveries;
pub use command::{LinkOptions, execute};
pub use deliveries::DeliveryRejection;

const JOURNAL: &str = "request.json";
const SECRET: &str = "recipient.key";

/// Distinct durable outcomes; cancelled and expired attempts never auto-resume.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// Waiting for an authenticated browser decision.
    Pending,
    /// Recoverable transport interruption, still bounded by the signed deadline.
    Interrupted,
    /// Explicit local cancellation; server publication may still have occurred.
    Cancelled,
    /// Initial approval window elapsed before local acceptance.
    Expired,
    /// Authenticated browser declined; no configuration was installed.
    Declined,
    /// Entire received approval is durable; replicas are being staged.
    Installing,
    /// The complete selection was atomically published.
    Completed,
}

/// One exact installed grant group and its isolated replica.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerminalSpace {
    /// Deterministic local alias.
    pub name: String,
    /// Isolated outer connection directory.
    pub site: PathBuf,
    /// Public binding selecting only this subject's grants.
    pub connection: ConnectionBinding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    request: String,
    origin: String,
    state: LinkState,
    approving_account: Option<String>,
    approval: Option<String>,
    spaces: Vec<TerminalSpace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conversion: Option<crate::account_session::ActiveAccount>,
    #[serde(default)]
    conversion_completed: bool,
}

/// Public request plus private local storage location. Debug excludes key bytes.
#[derive(Debug)]
pub struct TerminalLink {
    root: PathBuf,
    store: SpaceStore,
    request: LinkRequest,
    explicit_resume: bool,
}

fn regular_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= limit,
        "terminal journal contains an unsafe file"
    );
    Ok(std::fs::read(path)?)
}
fn journal(root: &Path) -> Result<Journal> {
    let value: Journal =
        serde_json::from_slice(&regular_bytes(&root.join(JOURNAL), 10 * 1024 * 1024)?)
            .context("terminal journal is malformed")?;
    ensure!(value.version == 1, "unsupported terminal journal");
    Ok(value)
}
fn save(root: &Path, journal: &Journal) -> Result<()> {
    connections::atomic_public(root, JOURNAL, &serde_json::to_vec_pretty(journal)?)
}
fn lock(root: &Path) -> Result<File> {
    connections::reject_unsafe_tree(root)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join("request.lock"))?;
    file.lock().context("failed to lock terminal request")?;
    Ok(file)
}
fn valid_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 64
            && id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid terminal request id"
    );
    Ok(())
}

// Management authorization authenticates publication at issued_at. A later
// consumer must enforce the current space grants, not prolong or require the
// independent management proof that signed the delivery descriptor.
async fn current_space_grants(
    bundles: &[tonk_invite::connection::SpaceGrantBundle],
    now: u64,
) -> Result<()> {
    let now = dialog_ucan_core::time::Timestamp::try_from(now as i128)?;
    for bundle in bundles {
        tonk_invite::connection::SpaceGrantBundle::validate(
            bundle.chains().to_vec(),
            bundle.recipient(),
            &tonk_invite::connection::candidate_build_scopes(bundle.subject()),
            bundle.remote(),
            now,
        )
        .await?;
    }
    Ok(())
}
async fn current_approval(bytes: &[u8], now: u64) -> Result<Approval> {
    let approval = Approval::inspect(bytes).await?;
    ensure!(
        approval.issued_at() <= now.saturating_add(30),
        "terminal approval is issued in the future"
    );
    current_space_grants(approval.bundles(), now).await?;
    Ok(approval)
}

impl TerminalLink {
    /// Create and fsync a fresh private key and signed request before opening a
    /// browser or sending any recipient-addressed network message.
    pub async fn create(
        store: SpaceStore,
        origin: &Url,
        service: &Did,
        label: &str,
        expected_account: Option<&Did>,
        now: u64,
    ) -> Result<Self> {
        ensure!(
            origin.path() == "/" && origin.query().is_none() && origin.fragment().is_none(),
            "terminal deployment must be a trusted origin"
        );
        let seed: [u8; 32] = rand::random();
        let signer = Signer::from(Ed25519Signer::import(&seed).await?);
        let request = LinkRequest::sign(
            &signer,
            service,
            rand::random(),
            now,
            now.checked_add(REQUEST_TTL_SECONDS)
                .context("invalid terminal deadline")?,
            label,
            expected_account,
        )
        .await?;
        request.to_url(origin.join("settings/link")?.as_str())?;
        let parent = store.root().join("terminal-links");
        connections::private_directory(&parent)?;
        let root = parent.join(request.id());
        ensure!(!root.exists(), "terminal request directory already exists");
        connections::private_directory(&root)?;
        connections::atomic_public(&root, SECRET, &seed)?;
        save(
            &root,
            &Journal {
                version: 1,
                request: hex::encode(request.bytes()),
                origin: origin.to_string(),
                state: LinkState::Pending,
                approving_account: None,
                approval: None,
                spaces: vec![],
                conversion: None,
                conversion_completed: false,
            },
        )?;
        Ok(Self {
            root: root.canonicalize()?,
            store,
            request,
            explicit_resume: false,
        })
    }

    /// Reopen the exact retained key and signed request without browser state.
    /// Explicit resume may fetch an approval durably published before its deadline;
    /// locally cancelled/expired attempts remain terminal and cannot resume.
    pub async fn resume(store: SpaceStore, id: &str, _now: u64) -> Result<Self> {
        valid_id(id)?;
        let root = store.root().join("terminal-links").join(id);
        let _lock = lock(&root)?;
        let mut saved = journal(&root)?;
        let request = LinkRequest::inspect(&hex::decode(&saved.request)?).await?;
        ensure!(
            request.id() == id,
            "terminal journal request binding changed"
        );
        let link = Self {
            root: root.canonicalize()?,
            store,
            request,
            explicit_resume: true,
        };
        link.signer().await?;
        match saved.state {
            LinkState::Cancelled | LinkState::Expired | LinkState::Declined => {
                anyhow::bail!("terminal request is {:?}; start a new link", saved.state)
            }
            _ => {}
        }
        link.recover_published(&mut saved).await?;
        link.recover_published_addition().await?;
        Ok(link)
    }
    /// Exact public request; safe to send to the browser and service.
    pub fn request(&self) -> &LinkRequest {
        &self.request
    }
    /// Approval URL with only signed public correlation information.
    pub fn url(&self) -> Result<Url> {
        let saved = journal(&self.root)?;
        self.request
            .to_url(Url::parse(&saved.origin)?.join("settings/link")?.as_str())
    }
    /// Current durable state.
    pub fn state(&self) -> Result<LinkState> {
        Ok(journal(&self.root)?.state)
    }
    /// Account authenticated by the accepted complete delivery, if one exists.
    pub fn approving_account(&self) -> Result<Option<String>> {
        Ok(journal(&self.root)?.approving_account)
    }
    /// Local signer; never serialize it into a browser request or delivery.
    pub async fn signer(&self) -> Result<Signer> {
        let seed: [u8; 32] = regular_bytes(&self.root.join(SECRET), 32)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("terminal private key is malformed"))?;
        let signer = Signer::from(Ed25519Signer::import(&seed).await?);
        ensure!(
            signer.did() == *self.request.recipient(),
            "terminal private key does not match request"
        );
        Ok(signer)
    }
    /// Record automatic approval timeout. Explicitly timed-out attempts do not resume.
    pub fn expire(&self) -> Result<()> {
        let _lock = lock(&self.root)?;
        let mut saved = journal(&self.root)?;
        if matches!(saved.state, LinkState::Pending | LinkState::Interrupted) {
            saved.state = LinkState::Expired;
            save(&self.root, &saved)?;
        }
        Ok(())
    }
    /// Record a recoverable transport interruption without altering configuration.
    pub fn interrupt(&self) -> Result<()> {
        let _lock = lock(&self.root)?;
        let mut saved = journal(&self.root)?;
        if saved.state == LinkState::Pending {
            saved.state = LinkState::Interrupted;
            save(&self.root, &saved)?;
        }
        Ok(())
    }
    /// Cancel locally. This cannot withdraw an approval published concurrently.
    pub fn cancel(&self) -> Result<()> {
        let _lock = lock(&self.root)?;
        let mut saved = journal(&self.root)?;
        ensure!(
            saved.state != LinkState::Completed,
            "completed terminal grants require explicit revocation, not cancellation"
        );
        let registry = self.store.load()?;
        ensure!(
            !saved.spaces.iter().any(|space| registry
                .spaces
                .get(&space.name)
                .is_some_and(|entry| entry.connection.as_ref() == Some(&space.connection))),
            "terminal selection was already published; resume to finish its journal"
        );
        saved.state = LinkState::Cancelled;
        save(&self.root, &saved)
    }

    /// Validate and durably accept one COMPLETE decision before installing any
    /// replica. `trusted_remotes` must come from independent service discovery.
    pub async fn accept(
        &self,
        bytes: &[u8],
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
    ) -> Result<Vec<TerminalSpace>> {
        self.accept_mode(bytes, trusted_remotes, now, false, || Ok(()))
            .await
    }
    /// Stage and confirm every selected space remotely before atomic publication.
    pub async fn accept_and_sync(
        &self,
        bytes: &[u8],
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
    ) -> Result<Vec<TerminalSpace>> {
        self.accept_mode(bytes, trusted_remotes, now, true, || Ok(()))
            .await
    }
    async fn accept_mode(
        &self,
        bytes: &[u8],
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
        sync: bool,
        after_publish: impl FnOnce() -> Result<()>,
    ) -> Result<Vec<TerminalSpace>> {
        let approval = current_approval(bytes, now).await?;
        ensure!(
            approval.request().bytes() == self.request.bytes(),
            "terminal approval is for a different request"
        );
        for bundle in approval.bundles() {
            ensure!(
                trusted_remotes.get(bundle.subject().as_str()) == Some(bundle.remote()),
                "terminal_untrusted_route"
            );
        }
        let _lock = lock(&self.root)?;
        let mut saved = journal(&self.root)?;
        ensure!(
            !matches!(
                saved.state,
                LinkState::Cancelled | LinkState::Expired | LinkState::Declined
            ),
            "terminal request is no longer accepting approval"
        );
        if matches!(saved.state, LinkState::Pending | LinkState::Interrupted)
            && now >= self.request.deadline()
            && !self.explicit_resume
        {
            saved.state = LinkState::Expired;
            save(&self.root, &saved)?;
            anyhow::bail!("terminal approval request expired before local acceptance");
        }
        let encoded = hex::encode(bytes);
        ensure!(
            saved
                .approval
                .as_ref()
                .is_none_or(|prior| prior == &encoded),
            "terminal approval conflicts with retained complete decision"
        );
        ensure!(
            saved
                .approving_account
                .as_ref()
                .is_none_or(|account| account == approval.account().as_str()),
            "terminal approving account changed"
        );
        if saved.state == LinkState::Completed {
            return Ok(saved.spaces);
        }
        saved.approving_account = Some(approval.account().to_string());
        saved.approval = Some(encoded);
        if approval.is_declined() {
            saved.state = LinkState::Declined;
            save(&self.root, &saved)?;
            anyhow::bail!("browser declined terminal connection; no spaces installed");
        }
        saved.state = LinkState::Installing;
        save(&self.root, &saved)?;
        self.install(&approval, &mut saved, sync, after_publish)
            .await
    }

    /// Resume an already accepted, complete durable decision after interruption.
    pub async fn finish(
        &self,
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
    ) -> Result<Vec<TerminalSpace>> {
        let saved = journal(&self.root)?;
        let approval = saved
            .approval
            .context("terminal request has no retained approval")?;
        self.accept(&hex::decode(approval)?, trusted_remotes, now)
            .await
    }

    /// Resume the exact durable complete decision, including remote confirmation.
    pub async fn finish_and_sync(
        &self,
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
    ) -> Result<Vec<TerminalSpace>> {
        let saved = journal(&self.root)?;
        let approval = saved
            .approval
            .context("terminal request has no retained approval")?;
        self.accept_and_sync(&hex::decode(approval)?, trusted_remotes, now)
            .await
    }

    async fn install(
        &self,
        approval: &Approval,
        saved: &mut Journal,
        sync: bool,
        after_publish: impl FnOnce() -> Result<()>,
    ) -> Result<Vec<TerminalSpace>> {
        let seed: [u8; 32] = regular_bytes(&self.root.join(SECRET), 32)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("terminal private key is malformed"))?;
        let now = dialog_ucan_core::time::Timestamp::now();
        let mut prepared = Vec::new();
        for bundle in approval.bundles() {
            let connection =
                ValidatedConnection::from_local_key(seed, bundle, bundle.remote(), now)
                    .await?
                    .with_terminal_request(&self.request.id())?;
            let binding = connection.binding().clone();
            let name = format!("link-{}", &binding.id[..16]);
            let site = self.root.join("replicas").join(&binding.id);
            connections::import_at(&site, &connection, self.store.clone()).await?;
            prepared.push(TerminalSpace {
                name,
                site: site.canonicalize()?,
                connection: binding,
            });
        }
        ensure!(
            saved.spaces.is_empty() || saved.spaces == prepared,
            "terminal prepared selection changed"
        );
        saved.spaces = prepared.clone();
        save(&self.root, saved)?;
        if sync {
            for space in &prepared {
                let site =
                    connections::open_bound(&space.site, &space.connection, self.store.clone())
                        .await?;
                crate::handoff::confirm_scoped_connection(&site, &space.connection.id).await?;
            }
        }
        self.publish_spaces(&prepared)?;
        after_publish()?;
        saved.state = LinkState::Completed;
        save(&self.root, saved)?;
        Ok(prepared)
    }

    // The registry publishes the complete selection only after installation and
    // (for the command) remote confirmation. Recover this final local checkpoint
    // historically: expired old authority must not block later fresh additions.
    async fn recover_published(&self, saved: &mut Journal) -> Result<()> {
        if saved.state != LinkState::Installing || saved.spaces.is_empty() {
            return Ok(());
        }
        let approval = Approval::inspect(&hex::decode(
            saved
                .approval
                .as_ref()
                .context("terminal published approval missing")?,
        )?)
        .await?;
        ensure!(
            approval.request().bytes() == self.request.bytes()
                && !approval.is_declined()
                && saved.approving_account.as_deref() == Some(approval.account().as_str())
                && saved.spaces.len() == approval.bundles().len(),
            "terminal published selection does not match its signed approval"
        );
        if !self
            .check_published_selection(&saved.spaces, approval.bundles())
            .await?
        {
            return Ok(());
        }
        let guard = self.store.write_guard()?;
        ensure!(
            Self::exact_registry_selection(&guard.load()?, &saved.spaces),
            "terminal published selection changed during recovery"
        );
        saved.state = LinkState::Completed;
        save(&self.root, saved)
    }
    fn exact_registry_selection(
        registry: &crate::space::Registry,
        spaces: &[TerminalSpace],
    ) -> bool {
        spaces.iter().all(|space| {
            registry.spaces.get(&space.name)
                == Some(&SpaceEntry {
                    site: space.site.clone(),
                    connection: Some(space.connection.clone()),
                })
                && !registry
                    .spaces
                    .iter()
                    .any(|(name, entry)| name != &space.name && entry.site == space.site)
        })
    }
    async fn check_published_selection(
        &self,
        spaces: &[TerminalSpace],
        bundles: &[tonk_invite::connection::SpaceGrantBundle],
    ) -> Result<bool> {
        ensure!(
            !spaces.is_empty() && spaces.len() == bundles.len(),
            "terminal complete published selection missing"
        );
        let registry = self.store.load()?;
        if !spaces.iter().any(|space| {
            registry.spaces.contains_key(&space.name)
                || registry
                    .spaces
                    .values()
                    .any(|entry| entry.site == space.site)
        }) {
            return Ok(false);
        }
        ensure!(
            Self::exact_registry_selection(&registry, spaces),
            "terminal published selection is incomplete or changed; no aliases were repaired"
        );
        for (space, bundle) in spaces.iter().zip(bundles) {
            let binding = connections::derive_binding(bundle);
            ensure!(
                space.connection == binding
                    && space.name == format!("link-{}", &binding.id[..16])
                    && space.site
                        == self
                            .root
                            .join("replicas")
                            .join(&binding.id)
                            .canonicalize()?,
                "terminal published replica differs from signed selection"
            );
            ensure!(
                connections::source_at(&space.site)?
                    == Some(connections::ConnectionSource::Terminal(self.request.id())),
                "terminal published replica source changed"
            );
            let _site =
                connections::open_published(&space.site, &binding, self.store.clone()).await?;
        }
        Ok(true)
    }
    fn publish_spaces(&self, prepared: &[TerminalSpace]) -> Result<()> {
        // No awaits under the registry guard. Validate every collision first;
        // then one atomic registry replacement publishes the entire selection.
        let guard = self.store.write_guard()?;
        let mut registry = guard.load()?;
        for space in prepared {
            crate::space::validate_name(&space.name)?;
            let expected = SpaceEntry {
                site: space.site.clone(),
                connection: Some(space.connection.clone()),
            };
            ensure!(
                registry
                    .spaces
                    .get(&space.name)
                    .is_none_or(|entry| entry == &expected),
                "terminal alias already belongs to other local data"
            );
            ensure!(
                !registry
                    .spaces
                    .iter()
                    .any(|(name, entry)| name != &space.name && entry.site == space.site),
                "terminal replica already has another alias"
            );
            ensure!(
                connections::binding_at(&space.site)?.as_ref() == Some(&space.connection),
                "terminal replica binding changed before publication"
            );
        }
        for space in prepared {
            registry.spaces.insert(
                space.name.clone(),
                SpaceEntry {
                    site: space.site.clone(),
                    connection: Some(space.connection.clone()),
                },
            );
        }
        guard.save(&registry)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject, time::Timestamp};
    use std::os::unix::fs::PermissionsExt;
    use tonk_invite::{
        connection::{SpaceGrantBundle, candidate_build_scopes},
        home_address_meta,
    };

    async fn key(seed: u8) -> Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap().into()
    }
    async fn decision(link: &TerminalLink, count: usize) -> Approval {
        decision_until(link, count, Timestamp::now().to_unix() + 90 * 86400).await
    }
    async fn decision_until(link: &TerminalLink, count: usize, deadline: u64) -> Approval {
        decision_with_deadlines(link, count, deadline, deadline).await
    }
    async fn decision_with_deadlines(
        link: &TerminalLink,
        count: usize,
        account_deadline: u64,
        space_deadline: u64,
    ) -> Approval {
        let account = key(2).await;
        let browser = key(3).await;
        let expiry = Timestamp::try_from(space_deadline as i128).unwrap();
        let account_proof = DelegationChain::new(
            DelegationBuilder::new()
                .issuer(account)
                .audience(&browser.did())
                .subject(Subject::Any)
                .command(vec![])
                .expiration(Timestamp::try_from(account_deadline as i128).unwrap())
                .try_build()
                .await
                .unwrap(),
        );
        let mut bundles = Vec::new();
        for index in 0..count {
            let space = key(10 + index as u8).await;
            let parent = DelegationBuilder::new()
                .issuer(space.clone())
                .audience(&browser.did())
                .subject(Subject::Specific(space.did()))
                .command(vec!["use".into()])
                .expiration(expiry)
                .try_build()
                .await
                .unwrap();
            let remote: Url = "https://tonk.network/ucan/".parse().unwrap();
            let scopes = candidate_build_scopes(&space.did());
            let mut chains = Vec::new();
            for scope in &scopes {
                let leaf = DelegationBuilder::new()
                    .issuer(browser.clone())
                    .audience(link.request.recipient())
                    .subject(scope.subject.clone())
                    .command(scope.command.0.clone())
                    .policy(scope.policy())
                    .expiration(expiry)
                    .meta(home_address_meta(&remote))
                    .try_build()
                    .await
                    .unwrap();
                chains.push(DelegationChain::new(parent.clone()).push(leaf).unwrap());
            }
            bundles.push(
                SpaceGrantBundle::validate(
                    chains,
                    link.request.recipient(),
                    &scopes,
                    &remote,
                    Timestamp::now(),
                )
                .await
                .unwrap(),
            );
        }
        Approval::sign(
            &browser,
            &link.request,
            account_proof,
            bundles,
            link.request.created_at() + 1,
        )
        .await
        .unwrap()
    }
    async fn pending(store: &SpaceStore, now: u64) -> TerminalLink {
        TerminalLink::create(
            store.clone(),
            &"https://tonk.network/".parse().unwrap(),
            &key(9).await.did(),
            "Local terminal",
            None,
            now,
        )
        .await
        .unwrap()
    }
    fn remotes(approval: &Approval) -> BTreeMap<String, Url> {
        approval
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect()
    }

    #[tokio::test]
    async fn one_selected_space_installs_accountlessly_with_durable_private_key() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        assert!(!store.registry_path().exists());
        assert_eq!(
            std::fs::metadata(link.root.join(SECRET))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&link.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let approval = decision(&link, 1).await;
        let installed = link
            .accept(approval.bytes(), &remotes(&approval), now + 2)
            .await
            .unwrap();
        assert_eq!(installed.len(), 1);
        assert!(store.load().unwrap().account.is_none());
        assert_eq!(link.state().unwrap(), LinkState::Completed);
        let reopened =
            connections::open_bound(&installed[0].site, &installed[0].connection, store.clone())
                .await
                .unwrap();
        assert!(reopened.is_scoped());
        let retained = TerminalLink::resume(store.clone(), &link.request.id(), now + 3)
            .await
            .unwrap();
        assert_eq!(
            retained.signer().await.unwrap().did(),
            *link.request.recipient()
        );
        assert_eq!(
            retained.finish(&remotes(&approval), now + 3).await.unwrap(),
            installed
        );
    }

    #[tokio::test]
    async fn cancelled_and_substituted_decisions_never_mutate_the_registry() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        store
            .set_account(Some(crate::space::AccountRecord::new("unrelated-account")))
            .unwrap();
        let original = std::fs::read(store.registry_path()).unwrap();
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let approval = decision(&link, 1).await;
        let other = pending(&store, now).await;
        assert!(
            other
                .accept(approval.bytes(), &remotes(&approval), now + 2)
                .await
                .is_err()
        );
        link.cancel().unwrap();
        assert!(
            link.accept(approval.bytes(), &remotes(&approval), now + 2)
                .await
                .is_err()
        );
        assert!(
            TerminalLink::resume(store.clone(), &link.request.id(), now + 3)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), original);
    }

    #[tokio::test]
    async fn explicit_interrupted_resume_accepts_timely_published_approval_after_window() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now - 601).await;
        let approval = decision(&link, 1).await;
        link.interrupt().unwrap();
        let resumed = TerminalLink::resume(store.clone(), &link.request.id(), now)
            .await
            .unwrap();
        // Models the trusted service's immutable delivery already published on
        // time. New service publication after the signed deadline is forbidden.
        assert_eq!(
            resumed
                .accept(approval.bytes(), &remotes(&approval), now)
                .await
                .unwrap()
                .len(),
            1
        );
        let expired = pending(&store, now - 601).await;
        expired.expire().unwrap();
        assert!(
            TerminalLink::resume(store, &expired.request.id(), now)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn whole_selection_collision_preserves_existing_account_and_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let approval = decision(&link, 2).await;
        let second = ValidatedConnection::from_local_key(
            regular_bytes(&link.root.join(SECRET), 32)
                .unwrap()
                .try_into()
                .unwrap(),
            &approval.bundles()[1],
            approval.bundles()[1].remote(),
            Timestamp::now(),
        )
        .await
        .unwrap();
        let collision = format!("link-{}", &second.binding().id[..16]);
        let mut registry = store.load().unwrap();
        registry.account = Some(crate::space::AccountRecord::new("retained-account"));
        registry.spaces.insert(
            collision,
            SpaceEntry::at(temp.path().join("unrelated-local-data")),
        );
        store.save(&registry).unwrap();
        let before = std::fs::read(store.registry_path()).unwrap();
        assert!(
            link.accept(approval.bytes(), &remotes(&approval), now + 2)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), before);
        assert_eq!(link.state().unwrap(), LinkState::Installing);
        link.cancel().unwrap();
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), before);
    }
    #[tokio::test]
    async fn offline_addition_pins_account_advances_cursor_and_retains_initial_replica() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let initial = decision(&link, 1).await;
        let original = link
            .accept(initial.bytes(), &remotes(&initial), now + 2)
            .await
            .unwrap();
        let expanded = decision(&link, 2).await;
        let added = expanded.bundles()[1].clone();
        let addition = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            expanded.account_proof().clone(),
            vec![added],
            [55; 32],
            now + 601,
        )
        .await
        .unwrap();
        let routes = addition
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        let installed = link
            .accept_addition(1, addition.bytes(), &routes, now + 602, false)
            .await
            .unwrap();
        assert_eq!(link.cursor().unwrap(), 1);
        assert_eq!(link.installed_spaces().unwrap().len(), 2);
        assert!(original[0].site.exists());
        assert_eq!(
            connections::source_at(&installed[0].site).unwrap(),
            Some(connections::ConnectionSource::Terminal(link.request.id()))
        );
        assert_eq!(
            link.accept_addition(1, addition.bytes(), &routes, now + 603, false)
                .await
                .unwrap(),
            installed
        );
        let config = crate::site::SiteConfig {
            account_store: store.clone(),
            ..crate::site::default_config().unwrap()
        };
        assert!(
            link.accept_addition(2, addition.bytes(), &routes, now + 604, false)
                .await
                .is_err()
        );
        assert_eq!(link.cursor().unwrap(), 1);
        let inventory = crate::inventory::list_local(&store, &config).await.unwrap();
        assert_eq!(inventory.rows.len(), 2, "{:?}", inventory.diagnostics);
        assert!(
            inventory
                .rows
                .iter()
                .all(|row| row.access_kind == crate::inventory::AccessKind::TerminalLinked)
        );
    }

    #[tokio::test]
    async fn published_checkpoint_recovers_after_expiry_without_healing_partial_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let initial = decision_until(&link, 2, now + 60).await;
        let error = link
            .accept_mode(initial.bytes(), &remotes(&initial), now + 2, false, || {
                anyhow::bail!("injected crash after complete registry publication")
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected crash"));
        assert_eq!(link.state().unwrap(), LinkState::Installing);
        let published = store.load().unwrap();
        assert_eq!(published.spaces.len(), 2);
        assert!(Approval::validate(initial.bytes(), now + 61).await.is_err());
        let removed = published.spaces.keys().next().unwrap().clone();
        let guard = store.write_guard().unwrap();
        let mut partial = guard.load().unwrap();
        partial.spaces.remove(&removed);
        guard.save(&partial).unwrap();
        drop(guard);
        let bytes_before = std::fs::read(store.registry_path()).unwrap();
        assert!(
            TerminalLink::resume(store.clone(), &link.request.id(), now + 61)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), bytes_before);
        let guard = store.write_guard().unwrap();
        guard.save(&published).unwrap();
        drop(guard);
        let bytes_before = std::fs::read(store.registry_path()).unwrap();
        let resumed = TerminalLink::resume(store.clone(), &link.request.id(), now + 61)
            .await
            .unwrap();
        assert_eq!(resumed.state().unwrap(), LinkState::Completed);
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), bytes_before);
        let expanded = decision(&resumed, 3).await;
        let addition = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            expanded.account_proof().clone(),
            vec![
                expanded
                    .bundles()
                    .iter()
                    .find(|bundle| {
                        !initial
                            .bundles()
                            .iter()
                            .any(|old| old.subject() == bundle.subject())
                    })
                    .unwrap()
                    .clone(),
            ],
            [91; 32],
            now + 61,
        )
        .await
        .unwrap();
        let routes = addition
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        resumed
            .accept_addition(1, addition.bytes(), &routes, now + 62, false)
            .await
            .unwrap();
        assert_eq!(resumed.cursor().unwrap(), 1);
        assert_eq!(resumed.installed_spaces().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn published_addition_recovers_expired_cursor_before_later_delivery() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let initial = decision(&link, 1).await;
        let original = link
            .accept(initial.bytes(), &remotes(&initial), now + 2)
            .await
            .unwrap();
        let expanded = decision_until(&link, 3, now + 60).await;
        let added: Vec<_> = expanded
            .bundles()
            .iter()
            .filter(|bundle| {
                !initial
                    .bundles()
                    .iter()
                    .any(|old| old.subject() == bundle.subject())
            })
            .cloned()
            .collect();
        let addition = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            expanded.account_proof().clone(),
            added,
            [92; 32],
            now + 2,
        )
        .await
        .unwrap();
        let routes = addition
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        let error = link
            .accept_addition_mode(1, addition.bytes(), &routes, now + 3, false, || {
                anyhow::bail!("injected crash before addition cursor commit")
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected crash"));
        assert_eq!(link.cursor().unwrap(), 0);
        assert!(link.pending_addition().unwrap().is_some());
        let published = store.load().unwrap();
        assert_eq!(published.spaces.len(), 3);
        assert!(
            tonk_invite::terminal::Addition::validate(addition.bytes(), now + 601)
                .await
                .is_err()
        );
        let remove = published
            .spaces
            .keys()
            .find(|name| **name != original[0].name)
            .unwrap()
            .clone();
        let guard = store.write_guard().unwrap();
        let mut partial = guard.load().unwrap();
        partial.spaces.remove(&remove);
        guard.save(&partial).unwrap();
        drop(guard);
        let before = std::fs::read(store.registry_path()).unwrap();
        assert!(
            TerminalLink::resume(store.clone(), &link.request.id(), now + 601)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), before);
        assert_eq!(link.cursor().unwrap(), 0);
        let guard = store.write_guard().unwrap();
        guard.save(&published).unwrap();
        drop(guard);
        let before = std::fs::read(store.registry_path()).unwrap();
        let resumed = TerminalLink::resume(store.clone(), &link.request.id(), now + 601)
            .await
            .unwrap();
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), before);
        assert_eq!(resumed.cursor().unwrap(), 1);
        assert!(resumed.pending_addition().unwrap().is_none());
        assert_eq!(resumed.installed_spaces().unwrap().len(), 3);
        let expanded = decision(&resumed, 4).await;
        let next_bundle = expanded
            .bundles()
            .iter()
            .find(|bundle| {
                !initial
                    .bundles()
                    .iter()
                    .chain(addition.bundles())
                    .any(|old| old.subject() == bundle.subject())
            })
            .unwrap()
            .clone();
        let next = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            expanded.account_proof().clone(),
            vec![next_bundle],
            [93; 32],
            now + 602,
        )
        .await
        .unwrap();
        let routes = next
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        resumed
            .accept_addition(2, next.bytes(), &routes, now + 603, false)
            .await
            .unwrap();
        assert_eq!(resumed.cursor().unwrap(), 2);
        assert_eq!(resumed.installed_spaces().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn expired_unpublished_addition_is_reported_and_fresh_readd_can_continue() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let initial = decision(&link, 1).await;
        link.accept(initial.bytes(), &remotes(&initial), now + 2)
            .await
            .unwrap();
        let expanded = decision_until(&link, 2, now + 60).await;
        let old_bundle = expanded
            .bundles()
            .iter()
            .find(|bundle| bundle.subject() != initial.bundles()[0].subject())
            .unwrap()
            .clone();
        let old = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            expanded.account_proof().clone(),
            vec![old_bundle.clone()],
            [94; 32],
            now + 2,
        )
        .await
        .unwrap();
        let routes = old
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        let binding = connections::derive_binding(&old_bundle);
        let occupied = format!("link-{}", &binding.id[..16]);
        let foreign = temp.path().join("foreign-local-data");
        std::fs::create_dir(&foreign).unwrap();
        let guard = store.write_guard().unwrap();
        let mut registry = guard.load().unwrap();
        registry.spaces.insert(
            occupied.clone(),
            SpaceEntry {
                site: foreign.clone(),
                connection: None,
            },
        );
        guard.save(&registry).unwrap();
        drop(guard);
        assert!(
            link.resolve_addition(1, old.bytes(), &routes, now + 3, false)
                .await
                .is_err()
        );
        assert_eq!(link.cursor().unwrap(), 0);
        assert!(link.rejected_deliveries().unwrap().is_empty());
        let staged = link.root.join("replicas").join(&binding.id);
        let site = connections::open_bound(&staged, &binding, store.clone())
            .await
            .unwrap();
        crate::handoff::record_scoped_connection(&site, &binding.id)
            .await
            .unwrap();
        let before_tree = site
            .branch()
            .await
            .unwrap()
            .handle()
            .revision()
            .unwrap()
            .tree
            .to_string();
        drop(site);
        let before_registry = std::fs::read(store.registry_path()).unwrap();
        link.resolve_addition(1, old.bytes(), &routes, now + 601, false)
            .await
            .unwrap();
        assert_eq!(link.cursor().unwrap(), 1);
        assert_eq!(
            link.rejected_deliveries().unwrap(),
            vec![(1, old.id(), DeliveryRejection::Expired)]
        );
        assert!(link.pending_addition().unwrap().is_none());
        assert_eq!(
            std::fs::read(store.registry_path()).unwrap(),
            before_registry
        );
        let site = connections::open_bound(&staged, &binding, store.clone())
            .await
            .unwrap();
        assert_eq!(
            site.branch()
                .await
                .unwrap()
                .handle()
                .revision()
                .unwrap()
                .tree
                .to_string(),
            before_tree
        );
        drop(site);
        let fresh = decision(&link, 2).await;
        let fresh_bundle = fresh
            .bundles()
            .iter()
            .find(|bundle| bundle.subject() == old_bundle.subject())
            .unwrap()
            .clone();
        assert_ne!(connections::derive_binding(&fresh_bundle).id, binding.id);
        let fresh = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            fresh.account_proof().clone(),
            vec![fresh_bundle],
            [95; 32],
            now + 602,
        )
        .await
        .unwrap();
        let routes = fresh
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        link.resolve_addition(2, fresh.bytes(), &routes, now + 603, false)
            .await
            .unwrap();
        assert_eq!(link.cursor().unwrap(), 2);
        assert_eq!(link.installed_spaces().unwrap().len(), 2);
        assert_eq!(store.load().unwrap().spaces[&occupied].site, foreign);
        let resumed = TerminalLink::resume(store, &link.request.id(), now + 604)
            .await
            .unwrap();
        assert_eq!(resumed.rejected_deliveries().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn expired_management_proof_does_not_expire_a_timely_published_shared_space() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let approval = decision_with_deadlines(&link, 1, now + 60, now + 90 * 86400).await;
        assert!(
            Approval::validate(approval.bytes(), now + 601)
                .await
                .is_err()
        );
        assert!(current_approval(approval.bytes(), now + 601).await.is_ok());
        link.interrupt().unwrap();
        let resumed = TerminalLink::resume(store.clone(), &link.request.id(), now + 601)
            .await
            .unwrap();
        let spaces = resumed
            .accept(approval.bytes(), &remotes(&approval), now + 601)
            .await
            .unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(resumed.state().unwrap(), LinkState::Completed);
        assert!(
            current_approval(approval.bytes(), now + 90 * 86400 + 1)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn expired_addition_management_proof_keeps_current_shared_space_grants() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let initial = decision(&link, 1).await;
        link.accept(initial.bytes(), &remotes(&initial), now + 2)
            .await
            .unwrap();
        let expanded = decision_with_deadlines(&link, 2, now + 60, now + 90 * 86400).await;
        let bundle = expanded
            .bundles()
            .iter()
            .find(|bundle| bundle.subject() != initial.bundles()[0].subject())
            .unwrap()
            .clone();
        let addition = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &initial,
            expanded.account_proof().clone(),
            vec![bundle],
            [96; 32],
            now + 3,
        )
        .await
        .unwrap();
        assert!(
            tonk_invite::terminal::Addition::validate(addition.bytes(), now + 601)
                .await
                .is_err()
        );
        let routes = addition
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        link.resolve_addition(1, addition.bytes(), &routes, now + 601, false)
            .await
            .unwrap();
        assert_eq!(link.cursor().unwrap(), 1);
        assert_eq!(link.installed_spaces().unwrap().len(), 2);
        assert!(link.rejected_deliveries().unwrap().is_empty());
        assert!(
            current_space_grants(addition.bundles(), now + 90 * 86400 + 1)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn addition_wrong_request_or_account_preserves_cursor_and_registry() {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path());
        let now = Timestamp::now().to_unix();
        let link = pending(&store, now).await;
        let initial = decision(&link, 1).await;
        link.accept(initial.bytes(), &remotes(&initial), now + 2)
            .await
            .unwrap();
        let other = pending(&store, now).await;
        let other_initial = decision(&other, 1).await;
        let expanded = decision(&other, 2).await;
        let addition = tonk_invite::terminal::Addition::sign(
            &key(3).await,
            &other_initial,
            expanded.account_proof().clone(),
            vec![expanded.bundles()[1].clone()],
            [56; 32],
            now + 3,
        )
        .await
        .unwrap();
        let before = std::fs::read(store.registry_path()).unwrap();
        let routes = addition
            .bundles()
            .iter()
            .map(|bundle| (bundle.subject().to_string(), bundle.remote().clone()))
            .collect();
        assert!(
            link.accept_addition(1, addition.bytes(), &routes, now + 4, false)
                .await
                .is_err()
        );
        assert_eq!(link.cursor().unwrap(), 0);
        assert_eq!(std::fs::read(store.registry_path()).unwrap(), before);
    }
}
