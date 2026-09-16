//! Durable, account-pinned ingestion of additions to completed terminal requests.
use super::*;
use tonk_invite::terminal::Addition;

const DELIVERIES: &str = "deliveries.json";
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliveryJournal {
    cursor: u64,
    applied: BTreeMap<u64, Applied>,
    pending: Option<Pending>,
    #[serde(default)]
    rejected: BTreeMap<u64, Rejected>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// A definitive rejected delivery, distinct from an installed local replica.
pub enum DeliveryRejection {
    /// At least one selected space grant passed its signed expiration.
    Expired,
    /// The configured service verified a standard UCAN revocation.
    Revoked,
}
impl DeliveryRejection {
    /// Stable human-readable outcome without claiming remote presence.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rejected {
    id: String,
    reason: DeliveryRejection,
    bytes: String,
    spaces: Vec<TerminalSpace>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Applied {
    id: String,
    spaces: Vec<TerminalSpace>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    sequence: u64,
    id: String,
    bytes: String,
    spaces: Vec<TerminalSpace>,
}
fn read(root: &Path) -> Result<DeliveryJournal> {
    match std::fs::symlink_metadata(root.join(DELIVERIES)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(DeliveryJournal::default())
        }
        Err(error) => Err(error.into()),
        Ok(_) => serde_json::from_slice(&regular_bytes(&root.join(DELIVERIES), 256 * 1024 * 1024)?)
            .context("terminal delivery journal is malformed"),
    }
}
fn save_deliveries(root: &Path, saved: &DeliveryJournal) -> Result<()> {
    connections::atomic_public(root, DELIVERIES, &serde_json::to_vec_pretty(saved)?)
}
impl TerminalLink {
    /// Resolve a delivery without allowing a permanently stale grant to block
    /// later fresh additions. Unknown or transient failures remain pending.
    pub async fn resolve_addition(
        &self,
        sequence: u64,
        bytes: &[u8],
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
        sync: bool,
    ) -> Result<()> {
        let addition = Addition::inspect(bytes).await?;
        for bundle in addition.bundles() {
            ensure!(
                trusted_remotes.get(bundle.subject().as_str()) == Some(bundle.remote()),
                "terminal_untrusted_route"
            );
        }
        if addition
            .bundles()
            .iter()
            .any(|bundle| bundle.expires_at().to_unix() <= now)
        {
            return self
                .reject_addition(sequence, bytes, DeliveryRejection::Expired, now)
                .await;
        }
        match self
            .accept_addition(sequence, bytes, trusted_remotes, now, sync)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let current = dialog_ucan_core::time::Timestamp::now().to_unix();
                let reason = match error.downcast_ref::<crate::sync::SyncError>() {
                    Some(crate::sync::SyncError::Rejected {
                        permanent: Some(crate::sync::PermanentRejection::Revoked),
                        ..
                    }) => Some(DeliveryRejection::Revoked),
                    Some(crate::sync::SyncError::Rejected {
                        permanent: Some(crate::sync::PermanentRejection::Expired),
                        ..
                    }) if addition
                        .bundles()
                        .iter()
                        .any(|bundle| bundle.expires_at().to_unix() <= current) =>
                    {
                        Some(DeliveryRejection::Expired)
                    }
                    _ => None,
                };
                match reason {
                    Some(reason) => self.reject_addition(sequence, bytes, reason, current).await,
                    None => Err(error),
                }
            }
        }
    }
    /// Last resolved sequence; only published or definitively rejected deliveries advance it.
    pub fn cursor(&self) -> Result<u64> {
        Ok(read(&self.root)?.cursor)
    }
    /// Retained delivery which must finish before polling later additions.
    pub fn pending_addition(&self) -> Result<Option<(u64, Vec<u8>)>> {
        read(&self.root)?
            .pending
            .map(|pending| Ok((pending.sequence, hex::decode(pending.bytes)?)))
            .transpose()
    }
    /// Durable terminal outcomes, kept distinct from installed grants.
    pub fn rejected_deliveries(&self) -> Result<Vec<(u64, String, DeliveryRejection)>> {
        Ok(read(&self.root)?
            .rejected
            .into_iter()
            .map(|(sequence, row)| (sequence, row.id, row.reason))
            .collect())
    }
    /// Every installed original or added group; removed access retains local data.
    pub fn installed_spaces(&self) -> Result<Vec<TerminalSpace>> {
        let mut spaces = journal(&self.root)?.spaces;
        for applied in read(&self.root)?.applied.into_values() {
            spaces.extend(applied.spaces);
        }
        Ok(spaces)
    }
    /// Verify a complete new delivery against the immutable original account and
    /// recipient, stage every replica, optionally confirm remote sync, then atomically
    /// publish its aliases and durably advance the transport-only cursor.
    pub async fn accept_addition(
        &self,
        sequence: u64,
        bytes: &[u8],
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
        sync: bool,
    ) -> Result<Vec<TerminalSpace>> {
        self.accept_addition_mode(sequence, bytes, trusted_remotes, now, sync, || Ok(()))
            .await
    }
    pub(super) async fn accept_addition_mode(
        &self,
        sequence: u64,
        bytes: &[u8],
        trusted_remotes: &BTreeMap<String, Url>,
        now: u64,
        sync: bool,
        after_publish: impl FnOnce() -> Result<()>,
    ) -> Result<Vec<TerminalSpace>> {
        let addition = Addition::inspect(bytes).await?;
        ensure!(
            addition.issued_at() <= now.saturating_add(30),
            "terminal addition is issued in the future"
        );
        current_space_grants(addition.bundles(), now).await?;
        ensure!(
            addition.request_id() == self.request.id()
                && addition.recipient() == self.request.recipient()
                && addition.service() == self.request.service(),
            "terminal addition request or recipient mismatch"
        );
        for bundle in addition.bundles() {
            ensure!(
                trusted_remotes.get(bundle.subject().as_str()) == Some(bundle.remote()),
                "terminal_untrusted_route"
            );
        }
        let _lock = lock(&self.root)?;
        let initial = journal(&self.root)?;
        ensure!(
            initial.state == LinkState::Completed,
            "terminal initial selection is not complete"
        );
        let initial_approval = Approval::inspect(&hex::decode(
            initial
                .approval
                .context("terminal initial approval missing")?,
        )?)
        .await?;
        ensure!(
            initial_approval.request().bytes() == self.request.bytes()
                && initial.approving_account.as_deref() == Some(addition.account().as_str())
                && initial_approval.account() == addition.account(),
            "terminal addition approving account mismatch"
        );
        let mut saved = read(&self.root)?;
        ensure!(
            !saved.rejected.contains_key(&sequence),
            "terminal delivery was already rejected"
        );
        if let Some(applied) = saved.applied.get(&sequence) {
            ensure!(
                applied.id == addition.id(),
                "terminal addition replay changed bytes"
            );
            return Ok(applied.spaces.clone());
        }
        ensure!(
            sequence > saved.cursor,
            "terminal addition sequence went backwards"
        );
        ensure!(
            !saved
                .applied
                .values()
                .any(|applied| applied.id == addition.id()),
            "terminal addition replay changed its delivery sequence"
        );
        ensure!(
            !saved.rejected.values().any(|row| row.id == addition.id()),
            "terminal rejected delivery changed its sequence"
        );
        if let Some(pending) = &saved.pending {
            ensure!(
                pending.sequence == sequence && pending.id == addition.id(),
                "finish retained terminal addition before accepting another"
            );
        } else {
            saved.pending = Some(Pending {
                sequence,
                id: addition.id(),
                bytes: hex::encode(bytes),
                spaces: vec![],
            });
            save_deliveries(&self.root, &saved)?;
        }
        let seed: [u8; 32] = regular_bytes(&self.root.join(SECRET), 32)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("terminal private key is malformed"))?;
        let mut prepared = Vec::new();
        for bundle in addition.bundles() {
            let connection = ValidatedConnection::from_local_key(
                seed,
                bundle,
                bundle.remote(),
                dialog_ucan_core::time::Timestamp::now(),
            )
            .await?
            .with_terminal_request(&self.request.id())?;
            let binding = connection.binding().clone();
            let site = self.root.join("replicas").join(&binding.id);
            connections::import_at(&site, &connection, self.store.clone()).await?;
            prepared.push(TerminalSpace {
                name: format!("link-{}", &binding.id[..16]),
                site: site.canonicalize()?,
                connection: binding,
            });
        }
        let pending = saved.pending.as_mut().expect("stored above");
        ensure!(
            pending.spaces.is_empty() || pending.spaces == prepared,
            "terminal addition prepared selection changed"
        );
        pending.spaces = prepared.clone();
        save_deliveries(&self.root, &saved)?;
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
        saved.applied.insert(
            sequence,
            Applied {
                id: addition.id(),
                spaces: prepared.clone(),
            },
        );
        saved.pending = None;
        saved.cursor = sequence;
        save_deliveries(&self.root, &saved)?;
        Ok(prepared)
    }
    // Called only while resume holds the request lock and has verified its key.
    pub(super) async fn recover_published_addition(&self) -> Result<()> {
        let mut saved = read(&self.root)?;
        let Some(pending) = &saved.pending else {
            return Ok(());
        };
        if pending.spaces.is_empty() {
            return Ok(());
        }
        let initial = journal(&self.root)?;
        ensure!(
            initial.state == LinkState::Completed,
            "terminal initial selection is incomplete"
        );
        let initial_approval = Approval::inspect(&hex::decode(
            initial
                .approval
                .context("terminal initial approval missing")?,
        )?)
        .await?;
        let addition = Addition::inspect(&hex::decode(&pending.bytes)?).await?;
        ensure!(
            pending.id == addition.id()
                && addition.request_id() == self.request.id()
                && addition.recipient() == self.request.recipient()
                && addition.service() == self.request.service()
                && initial_approval.request().bytes() == self.request.bytes()
                && initial_approval.account() == addition.account()
                && initial.approving_account.as_deref() == Some(addition.account().as_str()),
            "terminal retained addition binding changed"
        );
        ensure!(
            pending.sequence > saved.cursor
                && !saved.applied.contains_key(&pending.sequence)
                && !saved.rejected.contains_key(&pending.sequence)
                && !saved
                    .applied
                    .values()
                    .any(|applied| applied.id == pending.id),
            "terminal retained addition cursor changed"
        );
        if !self
            .check_published_selection(&pending.spaces, addition.bundles())
            .await?
        {
            return Ok(());
        }
        let guard = self.store.write_guard()?;
        ensure!(
            Self::exact_registry_selection(&guard.load()?, &pending.spaces),
            "terminal published addition changed during recovery"
        );
        saved.cursor = pending.sequence;
        saved.applied.insert(
            pending.sequence,
            Applied {
                id: pending.id.clone(),
                spaces: pending.spaces.clone(),
            },
        );
        saved.pending = None;
        save_deliveries(&self.root, &saved)
    }

    // The caller supplies Revoked only from a typed standard service decision.
    // Expiry is additionally established from the signed space-grant deadlines.
    pub(super) async fn reject_addition(
        &self,
        sequence: u64,
        bytes: &[u8],
        reason: DeliveryRejection,
        now: u64,
    ) -> Result<()> {
        let addition = Addition::inspect(bytes).await?;
        ensure!(
            addition.request_id() == self.request.id()
                && addition.recipient() == self.request.recipient()
                && addition.service() == self.request.service(),
            "terminal rejected delivery binding mismatch"
        );
        if reason == DeliveryRejection::Expired {
            ensure!(
                addition
                    .bundles()
                    .iter()
                    .any(|bundle| bundle.expires_at().to_unix() <= now),
                "terminal delivery has no expired space grant"
            );
        }
        let _lock = lock(&self.root)?;
        let initial = journal(&self.root)?;
        ensure!(
            initial.state == LinkState::Completed,
            "terminal initial selection incomplete"
        );
        let initial_approval = Approval::inspect(&hex::decode(
            initial
                .approval
                .context("terminal initial approval missing")?,
        )?)
        .await?;
        ensure!(
            initial_approval.request().bytes() == self.request.bytes()
                && initial_approval.account() == addition.account()
                && initial.approving_account.as_deref() == Some(addition.account().as_str()),
            "terminal rejected delivery account mismatch"
        );
        let mut saved = read(&self.root)?;
        if let Some(row) = saved.rejected.get(&sequence) {
            ensure!(
                row.id == addition.id() && row.reason == reason,
                "terminal rejected delivery replay changed"
            );
            return Ok(());
        }
        ensure!(
            sequence > saved.cursor
                && !saved.applied.contains_key(&sequence)
                && !saved.applied.values().any(|row| row.id == addition.id())
                && !saved.rejected.values().any(|row| row.id == addition.id()),
            "terminal rejected delivery cursor mismatch"
        );
        let spaces = match &saved.pending {
            Some(pending) => {
                ensure!(
                    pending.sequence == sequence && pending.id == addition.id(),
                    "another terminal delivery is pending"
                );
                pending.spaces.clone()
            }
            None => vec![],
        };
        saved.rejected.insert(
            sequence,
            Rejected {
                id: addition.id(),
                reason,
                bytes: hex::encode(bytes),
                spaces,
            },
        );
        saved.pending = None;
        saved.cursor = sequence;
        save_deliveries(&self.root, &saved)
    }
}
