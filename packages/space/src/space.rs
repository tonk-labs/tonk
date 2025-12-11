use crate::delegation::Delegation;
use crate::ownership::Ownership;
use dialog_artifacts::replica::{Branch, BranchId, Issuer, Replica};
use dialog_artifacts::selector::Constrained;
use dialog_artifacts::{Artifact, ArtifactSelector, ArtifactStore, DialogArtifactsError};
use dialog_query::claim::{Transaction, TransactionError};
use dialog_query::query::Source;
use dialog_query::{DeductiveRule, Session};
use dialog_storage::FileSystemStorageBackend;
use futures_core::Stream;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

/// Type alias for the filesystem-backed storage
pub type FsBackend = FileSystemStorageBackend<Vec<u8>, Vec<u8>>;

// Re-export types for CLI use
pub use dialog_artifacts::replica::{RemoteState, Revision, UpstreamState};
pub use dialog_storage::{AuthMethod, RestStorageConfig, S3Authority};

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
pub struct Space {
    /// The DID of this space
    pub did: String,
    /// The replica for managing remotes (wrapped in Arc<Mutex> for Clone)
    replica: Arc<Mutex<Replica<FsBackend>>>,
    /// The branch for this space (wrapped in Arc<Mutex> for Clone)
    branch: Arc<Mutex<Branch<FsBackend>>>,
    /// The session for querying and committing facts
    session: Session<Branch<FsBackend>>,
}

impl Space {
    /// Create a new space with the given parameters.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space (derived from space keypair by CLI)
    /// * `issuer` - The Issuer (operator) that will sign operations
    /// * `storage_path` - Path to store the space's facts (from CLI's add_space_to_session)
    /// * `delegations` - List of delegations to store in the space as ownership claims
    ///
    /// # Returns
    /// A new Space instance with the replica, branch, and delegations set up
    pub async fn create(
        space_did: String,
        issuer: Issuer,
        storage_path: PathBuf,
        delegations: Vec<Delegation>,
    ) -> Result<Self, SpaceError> {
        // Create the filesystem backend at the provided path
        let backend = FsBackend::new(&storage_path).await?;

        // Open the replica with the issuer
        let replica = Replica::open(issuer, backend)?;

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

        // Commit the transaction
        session.commit(transaction).await?;

        Ok(Space {
            did: space_did,
            replica: Arc::new(Mutex::new(replica)),
            branch: Arc::new(Mutex::new(branch)),
            session,
        })
    }

    /// Open an existing space.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space
    /// * `issuer` - The Issuer (operator) that will sign operations
    /// * `storage_path` - Path where the space's facts are stored
    ///
    /// # Returns
    /// The Space instance with access to the existing branch
    pub async fn open(
        space_did: String,
        issuer: Issuer,
        storage_path: PathBuf,
    ) -> Result<Self, SpaceError> {
        // Create the filesystem backend at the provided path
        let backend = FsBackend::new(&storage_path).await?;

        // Open the replica with the issuer
        let replica = Replica::open(issuer, backend)?;

        // Load the "main" branch
        let branch_id = BranchId::new("main".to_string());
        let branch = replica.branches.load(&branch_id).await?;

        // Create session for the branch (clone branch since Session takes ownership)
        let session = Session::open(branch.clone());

        Ok(Space {
            did: space_did,
            replica: Arc::new(Mutex::new(replica)),
            branch: Arc::new(Mutex::new(branch)),
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
            let mut replica = self.replica.lock().await;
            replica.remotes.add(remote_state).await?
        };

        // Open the remote branch (same branch ID as local)
        let branch_id = BranchId::new("main".to_string());
        let upstream = remote.open(&branch_id).await?;

        // Set the remote branch as upstream for our local branch
        {
            let mut branch = self.branch.lock().await;
            branch.set_upstream(upstream).await?;
        }

        Ok(())
    }

    /// Get the current revision of this space.
    pub async fn revision(&self) -> Revision {
        let branch = self.branch.lock().await;
        branch.revision()
    }

    /// Push local changes to the upstream remote.
    ///
    /// # Returns
    /// - `Ok(Some(old_revision))` if push succeeded and remote was updated
    /// - `Ok(None)` if there was nothing to push (already in sync)
    /// - `Err` if push failed or no upstream is configured
    pub async fn push(&mut self) -> Result<Option<Revision>, SpaceError> {
        let mut branch = self.branch.lock().await;
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
        let mut branch = self.branch.lock().await;
        let result = branch.pull().await?;
        Ok(result)
    }

    /// Get upstream info if configured.
    ///
    /// # Returns
    /// - `Some((site_name, branch_id, revision))` for remote upstream
    /// - `None` if no upstream is configured
    pub async fn upstream_info(&self) -> Option<(String, String, Option<Revision>)> {
        let branch = self.branch.lock().await;
        if let Some(upstream) = branch.upstream() {
            let site = upstream.site().map(|s| s.to_string()).unwrap_or_else(|| "local".to_string());
            let branch_id = upstream.id().to_string();
            let revision = upstream.revision();
            Some((site, branch_id, revision))
        } else {
            None
        }
    }

    /// Check if this space has an upstream configured.
    pub async fn has_upstream(&self) -> bool {
        let branch = self.branch.lock().await;
        branch.upstream().is_some()
    }
}

/// Implement ArtifactStore for Space by delegating to the inner session
impl ArtifactStore for Space {
    fn select(
        &self,
        artifact_selector: ArtifactSelector<Constrained>,
    ) -> impl Stream<Item = Result<Artifact, DialogArtifactsError>> + Send + 'static {
        self.session.select(artifact_selector)
    }
}

/// Implement Source for Space by delegating to the inner session
impl Source for Space {
    fn resolve_rules(&self, operator: &str) -> Vec<DeductiveRule> {
        self.session.resolve_rules(operator)
    }
}
