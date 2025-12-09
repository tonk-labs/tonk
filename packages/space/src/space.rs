use dialog_artifacts::replica::{Branch, BranchId, Issuer, Replica};
use dialog_query::claim::{Attribute, Claim, Relation, Transaction, TransactionError};
use dialog_query::{Entity, Session, Value};
use dialog_storage::FileSystemStorageBackend;
use std::path::PathBuf;
use std::str::FromStr;
use thiserror::Error;

/// Type alias for the filesystem-backed storage
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

/// Represents a delegation to be stored as facts in the space.
/// Implements `Claim` to be asserted via `Session::edit` API.
pub struct DelegationClaim {
    /// The CID of the delegation (used as entity)
    pub cid: String,
    /// The raw bytes of the delegation
    pub bytes: Vec<u8>,
    /// The issuer DID
    pub issuer: String,
    /// The audience DID
    pub audience: String,
    /// The subject DID (the space DID)
    pub subject: String,
    /// The command being delegated
    pub command: String,
}

impl Claim for DelegationClaim {
    fn assert(self, transaction: &mut Transaction) {
        // Parse entity from CID - if this fails, we skip (can't assert without valid entity)
        let entity = match Entity::from_str(&self.cid) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Helper to create relations - skip if attribute parsing fails
        let mut add_relation = |attr_str: &str, value: Value| {
            if let Ok(attr) = Attribute::from_str(attr_str) {
                Relation::new(attr, entity.clone(), value).assert(transaction);
            }
        };

        // Store the raw delegation bytes
        add_relation("db/blob", Value::Bytes(self.bytes));

        // Store the issuer DID
        add_relation("ucan/issuer", Value::String(self.issuer));

        // Store the audience DID
        add_relation("ucan/audience", Value::String(self.audience));

        // Store the subject DID
        add_relation("ucan/subject", Value::String(self.subject));

        // Store the command
        add_relation("ucan/cmd", Value::String(self.command));
    }

    fn retract(self, transaction: &mut Transaction) {
        // Parse entity from CID
        let entity = match Entity::from_str(&self.cid) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Helper to create relations for retraction
        let mut remove_relation = |attr_str: &str, value: Value| {
            if let Ok(attr) = Attribute::from_str(attr_str) {
                Relation::new(attr, entity.clone(), value).retract(transaction);
            }
        };

        remove_relation("db/blob", Value::Bytes(self.bytes));
        remove_relation("ucan/issuer", Value::String(self.issuer));
        remove_relation("ucan/audience", Value::String(self.audience));
        remove_relation("ucan/subject", Value::String(self.subject));
        remove_relation("ucan/cmd", Value::String(self.command));
    }
}

/// Represents space ownership - links a space DID to a delegation CID.
/// Implements `Claim` to be asserted via `Session::edit` API.
pub struct OwnerClaim {
    /// The space DID
    pub space_did: String,
    /// The delegation CID that grants ownership
    pub delegation_cid: String,
}

impl Claim for OwnerClaim {
    fn assert(self, transaction: &mut Transaction) {
        let entity = match Entity::from_str(&self.space_did) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Ok(attr) = Attribute::from_str("space/owner") {
            Relation::new(attr, entity, Value::String(self.delegation_cid)).assert(transaction);
        }
    }

    fn retract(self, transaction: &mut Transaction) {
        let entity = match Entity::from_str(&self.space_did) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Ok(attr) = Attribute::from_str("space/owner") {
            Relation::new(attr, entity, Value::String(self.delegation_cid)).retract(transaction);
        }
    }
}

/// Represents a Space - a collaboration unit backed by a dialog-db branch
pub struct Space {
    /// The DID of this space
    pub did: String,
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
    /// * `delegations` - List of delegations to store in the space
    ///
    /// # Returns
    /// A new Space instance with the replica, branch, and delegations set up
    pub async fn create(
        space_did: String,
        issuer: Issuer,
        storage_path: PathBuf,
        delegations: Vec<DelegationClaim>,
    ) -> Result<Self, SpaceError> {
        // Create the filesystem backend at the provided path
        let backend = FsBackend::new(&storage_path).await?;

        // Open the replica with the issuer
        let replica = Replica::open(issuer, backend)?;

        // Create/open the "main" branch for this space
        let branch_id = BranchId::new("main".to_string());
        let branch = replica.branches.open(&branch_id).await?;

        // Create session for the branch
        let mut session = Session::open(branch);

        // Build transaction with all delegations and owner claims
        let mut transaction = session.edit();

        for delegation in delegations {
            // Store owner claim linking space to this delegation
            let owner_claim = OwnerClaim {
                space_did: space_did.clone(),
                delegation_cid: delegation.cid.clone(),
            };

            // Assert both the delegation and ownership
            transaction.assert(delegation);
            transaction.assert(owner_claim);
        }

        // Commit the transaction
        session.commit(transaction).await?;

        Ok(Space {
            did: space_did,
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

        // Create session for the branch
        let session = Session::open(branch);

        Ok(Space {
            did: space_did,
            session,
        })
    }

    /// Add a new delegation to the space.
    pub async fn add_delegation(&mut self, delegation: DelegationClaim) -> Result<(), SpaceError> {
        let mut transaction = self.session.edit();

        // Store owner claim linking space to this delegation
        let owner_claim = OwnerClaim {
            space_did: self.did.clone(),
            delegation_cid: delegation.cid.clone(),
        };

        transaction.assert(delegation);
        transaction.assert(owner_claim);

        self.session.commit(transaction).await?;
        Ok(())
    }

    /// Get a reference to the session for querying
    pub fn session(&self) -> &Session<Branch<FsBackend>> {
        &self.session
    }
}
