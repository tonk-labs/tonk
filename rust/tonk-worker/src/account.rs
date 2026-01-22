//! User account storage as a dialog-db instance.
//!
//! The Account stores user-specific data like:
//! - Default space DID
//! - List of known space DIDs
//!
//! Unlike secrets (which go in KeyStore), this is non-sensitive metadata
//! that can be stored as regular dialog-db facts.

use dialog_query::{Attribute, Entity, With};
use thiserror::Error;
use tonk_space::{Operator, Space, SpaceError};

use crate::ServiceWorkerStorageBackend;

// Account-specific attributes
#[derive(Attribute, Clone, PartialEq)]
struct DefaultSpace(pub String);

#[derive(Attribute, Clone, PartialEq)]
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
    /// * `user_did` - The user's DID (used to name the database)
    /// * `operator` - The operator for signing account operations
    pub async fn open(user_did: &str, operator: &Operator) -> Result<Self, AccountError> {
        let db_name = format!("tonk-account:{}", user_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;

        // Open the space (creates if doesn't exist)
        let space = Space::open(user_did.to_string(), operator, backend).await?;

        Ok(Self { space })
    }

    /// Get the default space DID.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(did))` if a default space is set
    /// - `Ok(None)` if no default space has been set
    /// - `Err(...)` if the query fails
    pub async fn default_space(&self) -> Result<Option<String>, AccountError> {
        use dialog_query::concept::Match as _;
        use dialog_query::{Match, Term};
        use futures_util::TryStreamExt;

        let account_entity: Entity = ACCOUNT_ENTITY
            .parse()
            .map_err(|e| AccountError::Storage(format!("Failed to parse entity: {:?}", e)))?;

        let query = Match::<With<DefaultSpace>> {
            this: Term::from(account_entity),
            has: Term::var("default_space"),
        };

        let results: Vec<_> = query
            .query(self.space.clone())
            .try_collect()
            .await
            .map_err(|e| {
                AccountError::Storage(format!("Failed to query default space: {:?}", e))
            })?;

        Ok(results.first().map(|r| r.has.0.clone()))
    }

    /// Set the default space DID.
    pub async fn set_default_space(&mut self, space_did: &str) -> Result<(), AccountError> {
        tonk_common::log!("Setting default space to: {}", space_did);

        let account_entity: Entity = ACCOUNT_ENTITY
            .parse()
            .map_err(|e| AccountError::Storage(format!("Failed to parse entity: {:?}", e)))?;

        let mut transaction = self.space.edit();

        // Retract any existing default space
        if let Some(old_default) = self.default_space().await? {
            tonk_common::log!("Retracting old default space: {}", old_default);
            transaction.retract(With {
                this: account_entity.clone(),
                has: DefaultSpace(old_default),
            });
        }

        // Assert the new default space
        transaction.assert(With {
            this: account_entity,
            has: DefaultSpace(space_did.to_string()),
        });

        self.space.commit(transaction).await?;
        tonk_common::log!("Default space set successfully");
        Ok(())
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
    /// Does nothing if the space is already known.
    pub async fn add_known_space(&mut self, space_did: &str) -> Result<(), AccountError> {
        tonk_common::log!("Adding known space: {}", space_did);

        // Check if already known
        let known = self.known_spaces().await?;
        if known.contains(&space_did.to_string()) {
            tonk_common::log!("Space already known, skipping");
            return Ok(());
        }

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
