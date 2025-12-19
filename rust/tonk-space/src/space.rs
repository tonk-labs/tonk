use crate::delegation::Delegation;
use crate::operator::Operator;
use crate::ownership::Ownership;
use dialog_artifacts::replica::{Branch, BranchId, Issuer, Replica};
use dialog_artifacts::selector::Constrained;
use dialog_artifacts::{
    Artifact, ArtifactSelector, ArtifactStore, DialogArtifactsError, PlatformBackend,
};
use dialog_query::claim::{Transaction, TransactionError};
use dialog_query::query::Source;
use dialog_query::{DeductiveRule, Session};
use futures_core::Stream;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

// Re-export types for CLI use
pub use dialog_artifacts::replica::{RemoteState, Revision, UpstreamState};
pub use dialog_storage::{AuthMethod, MemoryStorageBackend, RestStorageConfig, S3Authority};

#[cfg(not(target_arch = "wasm32"))]
pub use dialog_storage::FileSystemStorageBackend;

/// Type alias for memory-backed storage (useful for tests)
pub type MemoryBackend = MemoryStorageBackend<Vec<u8>, Vec<u8>>;

/// Type alias for filesystem-backed storage (only available on native)
#[cfg(not(target_arch = "wasm32"))]
pub type FsBackend = FileSystemStorageBackend<Vec<u8>, Vec<u8>>;

/// Errors that can occur when working with spaces
#[derive(Debug, Error)]
pub enum SpaceError {
    #[error("Storage error: {0}")]
    Storage(#[from] dialog_storage::DialogStorageError),

    #[error("Replica error: {0}")]
    Replica(#[from] dialog_artifacts::replica::ReplicaError),

    #[error("Artifacts error: {0}")]
    Artifacts(#[from] dialog_artifacts::DialogArtifactsError),

    #[error("Transaction error: {0}")]
    Transaction(#[from] TransactionError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid entity: {0}")]
    InvalidEntity(String),

    #[error("Invalid attribute: {0}")]
    InvalidAttribute(String),
}

/// Represents a Space - a collaboration unit backed by a dialog-db branch
#[derive(Clone)]
pub struct Space<Backend: PlatformBackend + 'static> {
    /// The DID of this space
    pub did: String,
    /// The replica for managing remotes
    replica: Arc<RwLock<Replica<Backend>>>,
    /// The branch for this space
    branch: Arc<RwLock<Branch<Backend>>>,
    /// The session for querying and committing facts
    session: Session<Branch<Backend>>,
}

impl<Backend: PlatformBackend + 'static> Space<Backend> {
    /// Create a new space with the given parameters.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space (derived from space keypair by CLI)
    /// * `operator` - The operator that will sign operations
    /// * `backend` - The storage backend to use
    /// * `delegations` - List of delegations to store in the space as ownership claims
    ///
    /// # Returns
    /// A new Space instance with the replica, branch, and delegations set up
    pub async fn create(
        space_did: String,
        operator: &Operator,
        backend: Backend,
        delegations: Vec<Delegation>,
    ) -> Result<Self, SpaceError> {
        // Open the replica with the operator as issuer
        let replica = Replica::open(Issuer::from(operator), backend)?;

        // Create/open the "main" branch for this space
        let branch_id = BranchId::new("main".to_string());
        let branch = replica.branches.open(&branch_id).await?;

        // Create session for the branch (clone branch since Session takes ownership)
        let mut session = Session::open(branch.clone());

        // Build transaction with all ownership claims (which include delegations)
        let mut transaction = session.edit();

        for delegation in delegations {
            // Create ownership claim from delegation - this will assert both
            // the delegation facts and the space/owner relation
            transaction.assert(Ownership::from(delegation));
        }

        // Only commit if we have changes - empty transactions fail on new branches
        if !transaction.is_empty() {
            session.commit(transaction).await?;
        }

        Ok(Space {
            did: space_did,
            replica: Arc::new(RwLock::new(replica)),
            branch: Arc::new(RwLock::new(branch)),
            session,
        })
    }

    /// Open an existing space.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space
    /// * `operator` - The operator that will sign operations
    /// * `backend` - The storage backend to use
    ///
    /// # Returns
    /// The Space instance with access to the existing branch
    pub async fn open(
        space_did: String,
        operator: &Operator,
        backend: Backend,
    ) -> Result<Self, SpaceError> {
        // Open the replica with the operator as issuer
        let replica = Replica::open(Issuer::from(operator), backend)?;

        // Load the "main" branch
        let branch_id = BranchId::new("main".to_string());
        let branch = replica.branches.load(&branch_id).await?;

        // Create session for the branch (clone branch since Session takes ownership)
        let session = Session::open(branch.clone());

        Ok(Space {
            did: space_did,
            replica: Arc::new(RwLock::new(replica)),
            branch: Arc::new(RwLock::new(branch)),
            session,
        })
    }

    /// Create a new transaction for editing facts in this space.
    ///
    /// Returns a Transaction that can be used to assert or retract facts.
    /// Call `commit()` to persist the changes.
    pub fn edit(&self) -> Transaction {
        self.session.edit()
    }

    /// Commit a transaction to the space.
    ///
    /// Takes ownership of a Transaction and commits all its operations.
    pub async fn commit(&mut self, transaction: Transaction) -> Result<(), SpaceError> {
        self.session.commit(transaction).await?;
        Ok(())
    }

    /// Add a remote to this space and set it as upstream for the main branch.
    ///
    /// # Arguments
    /// * `remote_state` - Configuration for the remote (site name, S3 credentials, etc.)
    ///
    /// # Returns
    /// Ok(()) if the remote was added and set as upstream successfully.
    pub async fn add_remote(&mut self, remote_state: RemoteState) -> Result<(), SpaceError> {
        // Add the remote to the replica
        let remote = {
            let mut replica = self.replica.write().await;
            replica.remotes.add(remote_state).await?
        };

        // Open the remote branch (same branch ID as local)
        let branch_id = BranchId::new("main".to_string());
        let upstream = remote.open(&branch_id).await?;

        // Set the remote branch as upstream for our local branch
        {
            let mut branch = self.branch.write().await;
            branch.set_upstream(upstream).await?;
        }

        Ok(())
    }

    /// Get the current revision of this space.
    pub async fn revision(&self) -> Revision {
        let branch = self.branch.read().await;
        branch.revision()
    }

    /// Push local changes to the upstream remote.
    ///
    /// # Returns
    /// - `Ok(Some(old_revision))` if push succeeded and remote was updated
    /// - `Ok(None)` if there was nothing to push (already in sync)
    /// - `Err` if push failed or no upstream is configured
    pub async fn push(&mut self) -> Result<Option<Revision>, SpaceError> {
        let mut branch = self.branch.write().await;
        let result = branch.push().await?;
        Ok(result)
    }

    /// Pull changes from the upstream remote.
    ///
    /// # Returns
    /// - `Ok(Some(old_revision))` if pull succeeded and local was updated
    /// - `Ok(None)` if there was nothing to pull (already in sync)
    /// - `Err` if pull failed or no upstream is configured
    pub async fn pull(&mut self) -> Result<Option<Revision>, SpaceError> {
        let mut branch = self.branch.write().await;
        let result = branch.pull().await?;
        Ok(result)
    }

    /// Get upstream info if configured.
    ///
    /// # Returns
    /// - `Some((site_name, branch_id, revision))` for remote upstream
    /// - `None` if no upstream is configured
    pub async fn upstream_info(&self) -> Option<(String, String, Option<Revision>)> {
        let branch = self.branch.read().await;
        if let Some(upstream) = branch.upstream() {
            let site = upstream
                .site()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "local".to_string());
            let branch_id = upstream.id().to_string();
            let revision = upstream.revision();
            Some((site, branch_id, revision))
        } else {
            None
        }
    }

    /// Check if this space has an upstream configured.
    pub async fn has_upstream(&self) -> bool {
        let branch = self.branch.read().await;
        branch.upstream().is_some()
    }
}

/// Implement ArtifactStore for Space by delegating to the inner session
impl<Backend: PlatformBackend + 'static> ArtifactStore for Space<Backend> {
    #[allow(refining_impl_trait)]
    fn select(
        &self,
        artifact_selector: ArtifactSelector<Constrained>,
    ) -> impl Stream<Item = Result<Artifact, DialogArtifactsError>> + 'static {
        self.session.select(artifact_selector)
    }
}

/// Implement Source for Space by delegating to the inner session
impl<Backend: PlatformBackend + 'static> Source for Space<Backend> {
    fn resolve_rules(&self, operator: &str) -> Vec<DeductiveRule> {
        self.session.resolve_rules(operator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ucan::Delegation as UcanDelegation;
    use ucan::delegation::subject::DelegatedSubject;
    use ucan::did::Ed25519Signer;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    fn make_test_delegation() -> Delegation {
        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();

        let ucan_delegation = Delegation::builder()
            .issuer(Ed25519Signer::from(&issuer))
            .audience(audience.did().clone())
            .subject(DelegatedSubject::Specific(subject.did().clone()))
            .command(vec!["read".to_string(), "write".to_string()])
            .try_build()
            .expect("Failed to build delegation");

        Delegation::from(ucan_delegation)
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_creates_empty_space() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let space = Space::create(space_did.clone(), &operator, backend, vec![])
            .await
            .expect("Failed to create space");

        assert_eq!(space.did, space_did);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_creates_space_with_delegation() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();
        let delegation = make_test_delegation();

        let space = Space::create(space_did.clone(), &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        assert_eq!(space.did, space_did);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_opens_space_after_create() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();
        let delegation = make_test_delegation();

        // Create space first
        let _space = Space::create(
            space_did.clone(),
            &operator,
            backend.clone(),
            vec![delegation],
        )
        .await
        .expect("Failed to create space");

        // Now open the same space with the same operator
        let opened_space = Space::open(space_did.clone(), &operator, backend)
            .await
            .expect("Failed to open space");

        assert_eq!(opened_space.did, space_did);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_tracks_revision() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();
        let delegation = make_test_delegation();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        let revision = space.revision().await;
        // After one commit, we should have period 0 and moment > 0
        assert_eq!(revision.period, 0);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_has_no_upstream_by_default() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let space = Space::create(space_did, &operator, backend, vec![])
            .await
            .expect("Failed to create space");

        assert!(!space.has_upstream().await);
        assert!(space.upstream_info().await.is_none());
    }
}
