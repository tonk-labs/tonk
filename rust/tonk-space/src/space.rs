use crate::delegation::Delegation;
use crate::operator::Operator;
use crate::ownership::Ownership;
use dialog_artifacts::replica::{
    Branch, BranchId, Operator as ReplicaOperator, RemoteSite, Remotes, Replica,
};
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
pub use dialog_storage::MemoryStorageBackend;

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

    #[error("Query error: {0}")]
    Query(#[from] dialog_query::QueryError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid entity: {0}")]
    InvalidEntity(String),

    #[error("Invalid attribute: {0}")]
    InvalidAttribute(String),
}

/// Represents a Space - a collaboration unit backed by a dialog-db branch
///
/// The space is generic over the storage backend (e.g., IndexedDB, filesystem, memory).
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
        // Open the replica with the operator and space DID as subject
        let replica_operator = ReplicaOperator::from(operator);
        let replica = Replica::open(replica_operator, space_did.clone().into(), backend)?;

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

    /// Open an existing space, or create the branch if it doesn't exist.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space
    /// * `operator` - The operator that will sign operations
    /// * `backend` - The storage backend to use
    ///
    /// # Returns
    /// The Space instance with access to the branch
    pub async fn open(
        space_did: String,
        operator: &Operator,
        backend: Backend,
    ) -> Result<Self, SpaceError> {
        // Open the replica with the operator and space DID as subject
        let replica_operator = ReplicaOperator::from(operator);
        let replica = Replica::open(replica_operator, space_did.clone().into(), backend)?;

        // Open the "main" branch (creates it if it doesn't exist)
        let branch_id = BranchId::new("main".to_string());
        let branch = replica.branches.open(&branch_id).await?;

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

    /// Transact a set of changes to the space.
    ///
    /// Creates a transaction, applies all changes, and commits them atomically.
    /// This is a convenience method that combines `edit()` and `commit()`.
    pub async fn transact<E, D>(&mut self, changes: D) -> Result<(), SpaceError>
    where
        E: dialog_query::claim::Edit,
        D: IntoIterator<Item = E>,
    {
        let mut transaction = self.edit();
        for change in changes {
            change.merge(&mut transaction);
        }
        self.session.commit(transaction).await?;
        Ok(())
    }

    /// Add a remote to this space without setting it as upstream.
    ///
    /// # Arguments
    /// * `remote_state` - Configuration for the remote (site name, S3 credentials, etc.)
    ///
    /// # Returns
    /// The site name of the added remote.
    pub async fn add_remote(&mut self, remote_state: RemoteState) -> Result<String, SpaceError> {
        let mut replica = self.replica.write().await;
        let site = replica.add_remote(remote_state).await?;
        Ok(site)
    }

    /// Set a remote as upstream for the main branch.
    ///
    /// # Arguments
    /// * `site` - The site name of the remote to use as upstream
    ///
    /// # Returns
    /// Ok(()) if the upstream was set successfully.
    pub async fn set_upstream(&mut self, site: &str) -> Result<(), SpaceError> {
        // Load the remote site and get a reference to the remote branch
        let upstream = {
            let replica = self.replica.read().await;
            let remote_site = RemoteSite::load(
                &site.to_string(),
                replica.issuer().clone(),
                replica.storage().clone(),
            )
            .await?;
            remote_site.repository(self.did.clone()).branch("main")
        };

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

    /// Resolve a remote site and return its configuration info.
    ///
    /// # Arguments
    /// * `site` - The site name to resolve
    ///
    /// # Returns
    /// Site info if the site exists, including credentials details.
    pub async fn resolve_site(&self, site: &str) -> Result<SiteInfo, SpaceError> {
        let replica = self.replica.read().await;

        // Attempt to load the remote site - this will fail if it doesn't exist
        let remote_site = RemoteSite::load(
            &site.to_string(),
            replica.issuer().clone(),
            replica.storage().clone(),
        )
        .await?;

        // Get state to extract credentials info
        let credentials_info = remote_site
            .state()
            .map(|state| CredentialsInfo::from_credentials(&state.credentials));

        Ok(SiteInfo {
            name: site.to_string(),
            credentials: credentials_info,
        })
    }

    /// Get info about a branch.
    ///
    /// # Arguments
    /// * `branch_name` - The branch name to query (use "main" for the default branch)
    ///
    /// # Returns
    /// Branch info including revision and upstream state.
    pub async fn branch_info(&self, branch_name: &str) -> Result<BranchInfo, SpaceError> {
        let replica = self.replica.read().await;
        let branch_id = BranchId::new(branch_name.to_string());
        let branch = replica.branches.open(&branch_id).await?;

        let revision = branch.revision();
        let base = format!("{}", branch.base());
        let upstream = branch.upstream().map(|u| UpstreamInfo {
            site: u.site().map(|s| s.to_string()),
            branch: u.id().to_string(),
            revision: u.revision(),
        });

        Ok(BranchInfo {
            name: branch_name.to_string(),
            revision,
            base,
            upstream,
        })
    }

    /// Resolve a remote branch and return its revision.
    ///
    /// This actually connects to the remote and fetches the current revision,
    /// which validates the credentials and network connectivity.
    ///
    /// # Arguments
    /// * `site` - The remote site name
    /// * `repo_did` - The repository DID (subject)
    /// * `branch_name` - The branch name
    ///
    /// # Returns
    /// Remote branch info including the resolved revision.
    pub async fn resolve_remote_branch(
        &self,
        site: &str,
        repo_did: &str,
        branch_name: &str,
    ) -> Result<RemoteBranchInfo, SpaceError> {
        let replica = self.replica.read().await;

        // Load the remote site
        let remote_site = RemoteSite::load(
            &site.to_string(),
            replica.issuer().clone(),
            replica.storage().clone(),
        )
        .await?;

        // Get the remote branch reference
        let mut remote_branch = remote_site
            .repository(repo_did.to_string())
            .branch(branch_name);

        // Resolve the remote branch - this actually connects to the remote
        let revision = remote_branch.resolve().await?;

        Ok(RemoteBranchInfo {
            site: site.to_string(),
            repo_did: repo_did.to_string(),
            branch: branch_name.to_string(),
            revision,
        })
    }

    /// Query all delegations where the given DID is the audience.
    ///
    /// This is useful for finding all delegations that grant authority to a
    /// specific user. The returned delegations can be used to reconstruct
    /// authorization chains for UCAN verification.
    ///
    /// # Arguments
    /// * `audience_did` - The DID of the audience to query for
    ///
    /// # Returns
    /// A vector of delegations where the audience matches the given DID
    pub async fn delegations_for_audience(
        &self,
        audience_did: &str,
    ) -> Result<Vec<Delegation>, SpaceError> {
        use crate::schema;
        use dialog_query::concept::Match as _;
        use dialog_query::{Match, Term, With};
        use futures_util::TryStreamExt;

        // Query for entities with the specified audience
        let query = Match::<With<schema::ucan::Audience>> {
            this: Term::var("delegation"),
            has: Term::from(audience_did.to_string()),
        };

        let results: Vec<_> = query.query(self.clone()).try_collect().await?;

        // For each match, fetch the blob and deserialize to Delegation
        let mut delegations = Vec::new();
        for result in results {
            let blob_query = Match::<With<schema::ucan::Blob>> {
                this: Term::from(result.this),
                has: Term::var("blob"),
            };

            let blobs: Vec<_> = blob_query.query(self.clone()).try_collect().await?;
            if let Some(blob_result) = blobs.first() {
                let delegation: Delegation = serde_ipld_dagcbor::from_slice(&blob_result.has.0)
                    .map_err(|e| SpaceError::InvalidEntity(e.to_string()))?;
                delegations.push(delegation);
            }
        }

        Ok(delegations)
    }
}

/// Information about a resolved remote branch.
#[derive(Clone, Debug)]
pub struct RemoteBranchInfo {
    /// The site name
    pub site: String,
    /// The repository DID
    pub repo_did: String,
    /// The branch name
    pub branch: String,
    /// The resolved revision (None if branch doesn't exist on remote)
    pub revision: Option<Revision>,
}

/// Information about a remote site.
#[derive(Clone, Debug)]
pub struct SiteInfo {
    /// The site name
    pub name: String,
    /// Credentials info (None if state couldn't be loaded)
    pub credentials: Option<CredentialsInfo>,
}

/// Information about credentials configuration.
#[derive(Clone, Debug)]
pub enum CredentialsInfo {
    /// S3-based credentials
    S3 {
        /// The S3 region
        region: String,
        /// The S3 bucket name
        bucket: String,
        /// Whether private (signed) access is configured
        is_private: bool,
    },
    /// UCAN-based credentials
    Ucan {
        /// The access service endpoint
        service_url: String,
        /// The audience DID (operator)
        audience_did: String,
        /// The subject DID (from delegation)
        subject_did: Option<String>,
        /// The command scope
        command: Option<String>,
    },
}

impl CredentialsInfo {
    /// Create credentials info from dialog-artifacts RemoteCredentials
    pub fn from_credentials(credentials: &dialog_artifacts::replica::RemoteCredentials) -> Self {
        match credentials {
            dialog_artifacts::replica::RemoteCredentials::S3(s3_creds) => {
                let is_private =
                    matches!(s3_creds, dialog_s3_credentials::s3::Credentials::Private(_));
                CredentialsInfo::S3 {
                    region: s3_creds.region().to_string(),
                    bucket: s3_creds.bucket().to_string(),
                    is_private,
                }
            }
            dialog_artifacts::replica::RemoteCredentials::Ucan(ucan_creds) => {
                let delegation = ucan_creds.delegation();

                CredentialsInfo::Ucan {
                    service_url: ucan_creds.endpoint().to_string(),
                    audience_did: ucan_creds.audience().to_string(),
                    subject_did: delegation.subject().map(|d| d.to_string()),
                    command: Some(delegation.ability()),
                }
            }
            dialog_artifacts::replica::RemoteCredentials::Memory => {
                // Memory credentials don't have meaningful info to display
                CredentialsInfo::S3 {
                    region: "memory".to_string(),
                    bucket: "memory".to_string(),
                    is_private: false,
                }
            }
        }
    }
}

/// Information about a branch.
#[derive(Clone, Debug)]
pub struct BranchInfo {
    /// The branch name
    pub name: String,
    /// Current revision
    pub revision: Revision,
    /// Base tree hash (the tree we're based off for tracking local changes)
    pub base: String,
    /// Upstream info if configured
    pub upstream: Option<UpstreamInfo>,
}

/// Information about upstream configuration.
#[derive(Clone, Debug)]
pub struct UpstreamInfo {
    /// The site name (None for local upstream)
    pub site: Option<String>,
    /// The branch name on the upstream
    pub branch: String,
    /// The upstream revision
    pub revision: Option<Revision>,
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
    use crate::schema;
    use dialog_query::concept::Match as _;
    use dialog_query::{Match, Term, With};
    use futures_util::TryStreamExt;
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
            .audience(*audience.did())
            .subject(DelegatedSubject::Specific(*subject.did()))
            .command(vec!["read".to_string(), "write".to_string()])
            .try_build()
            .expect("Failed to build delegation");

        Delegation::from(ucan_delegation)
    }

    fn make_delegation_with_parts(
        issuer: &Operator,
        audience: &Operator,
        subject: &Operator,
    ) -> Delegation {
        let ucan_delegation = Delegation::builder()
            .issuer(Ed25519Signer::from(issuer))
            .audience(*audience.did())
            .subject(DelegatedSubject::Specific(*subject.did()))
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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_stores_delegation_issuer() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let delegation_entity = delegation.this();
        let expected_issuer = issuer.did().to_string();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for the issuer attribute on the delegation entity
        let query = Match::<With<schema::ucan::Issuer>> {
            this: Term::from(delegation_entity),
            has: Term::var("issuer"),
        };

        let results: Vec<_> = query.query(space.clone()).try_collect().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].has.0, expected_issuer);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_stores_delegation_audience() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let ucan = delegation.this();
        let expected_audience = audience.did().to_string();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for the audience attribute on the delegation entity
        let query = Match::<With<schema::ucan::Audience>> {
            this: Term::from(ucan),
            has: Term::var("audience"),
        };

        let results: Vec<_> = query.query(space.clone()).try_collect().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].has.0, expected_audience);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_stores_delegation_subject() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let ucan = delegation.this();
        let expected_subject = subject.did().to_string();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for the subject attribute on the delegation entity
        let query = Match::<With<schema::ucan::Subject>> {
            this: Term::from(ucan),
            has: Term::var("subject"),
        };

        let results: Vec<_> = query.query(space.clone()).try_collect().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].has.0, expected_subject);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_stores_delegation_command() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let ucan = delegation.this();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for the cmd attribute on the delegation entity
        let query = Match::<With<schema::ucan::Cmd>> {
            this: Term::from(ucan),
            has: Term::var("cmd"),
        };

        let results: Vec<_> = query.query(space.clone()).try_collect().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].has.0, "/read/write");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_stores_space_owner() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let ucan = delegation.this();
        let space_entity = subject.did().to_string();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for the owner attribute on the space (subject) entity
        let query = Match::<With<schema::space::Owner>> {
            this: Term::from(
                space_entity
                    .parse::<dialog_query::Entity>()
                    .expect("valid entity"),
            ),
            has: Term::var("owner"),
        };

        let results: Vec<_> = query.query(space.clone()).try_collect().await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].has.0, ucan);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_can_query_delegations_by_issuer() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let issuer_did = issuer.did().to_string();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for any entity with this specific issuer
        let query = Match::<With<schema::ucan::Issuer>> {
            this: Term::var("delegation"),
            has: Term::from(issuer_did),
        };

        let results: Vec<_> = query.query(space.clone()).try_collect().await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_queries_delegations_for_audience() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();

        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let delegation = make_delegation_with_parts(&issuer, &audience, &subject);
        let audience_did = audience.did().to_string();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query using the new helper method
        let delegations = space.delegations_for_audience(&audience_did).await.unwrap();
        assert_eq!(delegations.len(), 1);
        assert_eq!(delegations[0].audience().to_string(), audience_did);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_returns_empty_for_unknown_audience() {
        let backend = MemoryBackend::default();
        let space_did = "did:key:z6MktRgfR4aqompSzCHvmwCxERDjWyn2QDXURd1vdqBgMozV".to_string();
        let operator = Operator::generate();
        let delegation = make_test_delegation();

        let space = Space::create(space_did, &operator, backend, vec![delegation])
            .await
            .expect("Failed to create space");

        // Query for an audience that doesn't exist
        let unknown_audience = "did:key:z6MkUnknownAudienceXXXXXXXXXXXXXXXXXXXXXXXXXX";
        let delegations = space
            .delegations_for_audience(unknown_audience)
            .await
            .unwrap();
        assert!(delegations.is_empty());
    }
}
