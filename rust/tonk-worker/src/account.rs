//! User account storage as a dialog-db instance.
//!
//! The Account stores user-specific data like the list of known space DIDs.
//!
//! Unlike secrets (which go in KeyStore), this is non-sensitive metadata
//! that can be stored as regular dialog-db facts.
//!
//! TODO: Consider replacing known_spaces with UCAN delegation queries. Instead
//! of maintaining a separate list, we could query for delegations where the
//! account is the audience to discover accessible spaces.

use dialog_query::{Attribute, Entity, With};
use thiserror::Error;
use tonk_space::{Operator, Space, SpaceError};

use crate::ServiceWorkerStorageBackend;

/// A space DID that this account has access to.
/// Multiple spaces can be associated with a single account.
#[derive(Attribute, Clone, PartialEq)]
#[cardinality(many)]
struct KnownSpace(pub String);

/// Errors that can occur when working with the account.
#[derive(Debug, Error)]
pub enum AccountError {
    /// Space operation failed.
    #[error("Space error: {0}")]
    Space(#[from] SpaceError),

    /// Storage error.
    #[error("Storage error: {0}")]
    Storage(String),
}

/// The account entity - a fixed entity ID for storing account-level facts.
/// Must be a valid URI (urn: scheme works well for fixed identifiers).
const ACCOUNT_ENTITY: &str = "urn:tonk:account";

/// User account storage backed by dialog-db.
///
/// Stores user preferences and metadata as facts in a dedicated space.
#[derive(Clone)]
pub struct Account {
    space: Space<ServiceWorkerStorageBackend>,
}

impl Account {
    /// Open or create the account for the given user.
    ///
    /// # Arguments
    /// * `db_name` - The database name (should include tonk: prefix for debug clarity)
    /// * `operator` - The operator for signing account operations
    pub async fn open(db_name: &str, operator: &Operator) -> Result<Self, AccountError> {
        let backend = ServiceWorkerStorageBackend::new(db_name).await;

        // Open the space (creates if doesn't exist)
        let space = Space::open(db_name.to_string(), operator, backend).await?;

        Ok(Self { space })
    }

    /// Get all known space DIDs.
    ///
    /// # Returns
    ///
    /// - `Ok(vec)` with the list of known space DIDs (may be empty)
    /// - `Err(...)` if the query fails
    pub async fn known_spaces(&self) -> Result<Vec<String>, AccountError> {
        use dialog_query::concept::Match as _;
        use dialog_query::{Match, Term};
        use futures_util::TryStreamExt;

        let account_entity: Entity = ACCOUNT_ENTITY
            .parse()
            .map_err(|e| AccountError::Storage(format!("Failed to parse entity: {:?}", e)))?;

        let query = Match::<With<KnownSpace>> {
            this: Term::from(account_entity),
            has: Term::var("known_space"),
        };

        let results: Vec<_> = query
            .query(self.space.clone())
            .try_collect()
            .await
            .map_err(|e| AccountError::Storage(format!("Failed to query known spaces: {:?}", e)))?;

        Ok(results.into_iter().map(|r| r.has.0).collect())
    }

    /// Add a space to the list of known spaces.
    ///
    /// With cardinality(many), dialog-db handles deduplication automatically.
    pub async fn add_known_space(&mut self, space_did: &str) -> Result<(), AccountError> {
        tonk_common::log!("Adding known space: {}", space_did);

        let account_entity: Entity = ACCOUNT_ENTITY
            .parse()
            .map_err(|e| AccountError::Storage(format!("Failed to parse entity: {:?}", e)))?;

        let mut transaction = self.space.edit();
        transaction.assert(With {
            this: account_entity,
            has: KnownSpace(space_did.to_string()),
        });

        self.space.commit(transaction).await?;
        tonk_common::log!("Known space added successfully");
        Ok(())
    }

    /// Remove a space from the list of known spaces.
    pub async fn remove_known_space(&mut self, space_did: &str) -> Result<(), AccountError> {
        tonk_common::log!("Removing known space: {}", space_did);

        let account_entity: Entity = ACCOUNT_ENTITY
            .parse()
            .map_err(|e| AccountError::Storage(format!("Failed to parse entity: {:?}", e)))?;

        let mut transaction = self.space.edit();
        transaction.retract(With {
            this: account_entity,
            has: KnownSpace(space_did.to_string()),
        });

        self.space.commit(transaction).await?;
        tonk_common::log!("Known space removed successfully");
        Ok(())
    }
}
