//! Workspace - an opened space with user context.
//!
//! A Workspace represents an active session where a user is working with a specific
//! space. It combines the space data with the user's identity context, enabling
//! operations that require both (like querying delegations granted to the user).

use thiserror::Error;
use tonk_space::{Delegation, Operator, SpaceError};
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::Ed25519Signer;

use crate::ServiceWorkerStorageBackend;
use crate::account::AccountError;
use crate::identity::Identity;
use crate::key_store::KeyStoreError;
use tonk_space::Space;

/// Errors that can occur when working with workspaces.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// No default space has been set.
    #[error("No default space configured")]
    NoDefaultSpace,

    /// The requested space was not found.
    #[error("Space not found: {0}")]
    SpaceNotFound(String),

    /// Failed to access key store.
    #[error("Key store error: {0}")]
    KeyStore(#[from] KeyStoreError),

    /// Failed to access account.
    #[error("Account error: {0}")]
    Account(#[from] AccountError),

    /// Failed to operate on space.
    #[error("Space error: {0}")]
    Space(#[from] SpaceError),

    /// Failed to create delegation.
    #[error("Delegation error: {0}")]
    Delegation(String),
}

/// An active workspace - a space opened by a specific user.
pub struct Workspace {
    /// The user's DID.
    user_did: String,
    /// The opened space.
    space: Space<ServiceWorkerStorageBackend>,
    /// The space's operator (for signing space-level operations).
    space_operator: Operator,
}

impl Workspace {
    /// Open an existing space (or the default space if space_did is None).
    pub(crate) async fn open(
        identity: &Identity,
        space_did: Option<&str>,
    ) -> Result<Self, WorkspaceError> {
        let space_did = match space_did {
            Some(did) => did.to_string(),
            None => identity
                .account()
                .default_space()
                .await?
                .ok_or(WorkspaceError::NoDefaultSpace)?,
        };

        // Get the space operator from key store
        let space_operator = identity
            .key_store()
            .space_operator(&space_did)
            .await?
            .ok_or_else(|| WorkspaceError::SpaceNotFound(space_did.clone()))?;

        // Open the space database
        let db_name = format!("tonk-space:{}", space_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;
        let space = Space::open(space_did, &space_operator, backend).await?;

        Ok(Self {
            user_did: identity.did().to_string(),
            space,
            space_operator,
        })
    }

    /// Create a new space owned by this user.
    pub(crate) async fn create(identity: &mut Identity) -> Result<Self, WorkspaceError> {
        // Generate a new keypair for the space
        let space_operator = identity.key_store().create_space_operator().await?;
        let space_did = space_operator.did().to_string();

        // Store space operator in key store
        identity
            .key_store()
            .store_space_operator(&space_did, &space_operator)
            .await?;

        // Create delegation: space -> user (space grants user full authority)
        let delegation = Self::create_ownership_delegation(&space_operator, identity.operator())?;

        // Update account with the new space
        identity.account_mut().set_default_space(&space_did).await?;
        identity.account_mut().add_known_space(&space_did).await?;

        // Create the space database and space with ownership delegation
        let db_name = format!("tonk-space:{}", space_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;
        let space = Space::create(space_did, &space_operator, backend, vec![delegation]).await?;

        Ok(Self {
            user_did: identity.did().to_string(),
            space,
            space_operator,
        })
    }

    /// Create an ownership delegation from space to user.
    fn create_ownership_delegation(
        space_operator: &Operator,
        user_operator: &Operator,
    ) -> Result<Delegation, WorkspaceError> {
        let ucan_delegation = Delegation::builder()
            .issuer(Ed25519Signer::from(space_operator))
            .audience(*user_operator.did())
            .subject(DelegatedSubject::Specific(*space_operator.did()))
            .command(vec![]) // Empty command = "/*" (all commands)
            .try_build()
            .expect("Delegation builder should not fail with valid inputs");

        Ok(Delegation::from(ucan_delegation))
    }

    // === Accessors ===

    /// Get the user's DID.
    pub fn user_did(&self) -> &str {
        &self.user_did
    }

    /// Get the space's DID.
    pub fn space_did(&self) -> &str {
        &self.space.did
    }

    /// Get a reference to the space.
    pub fn space(&self) -> &Space<ServiceWorkerStorageBackend> {
        &self.space
    }

    /// Get a mutable reference to the space.
    pub fn space_mut(&mut self) -> &mut Space<ServiceWorkerStorageBackend> {
        &mut self.space
    }

    /// Get the space operator (for signing space-level operations).
    pub fn space_operator(&self) -> &Operator {
        &self.space_operator
    }

    // === Operations requiring both user and space context ===

    /// Get all delegations granted to this user for this space.
    pub async fn user_delegations(&self) -> Result<Vec<Delegation>, WorkspaceError> {
        Ok(self.space.delegations_for_audience(&self.user_did).await?)
    }

    /// Check if the current user owns this space.
    pub async fn user_is_owner(&self) -> Result<bool, WorkspaceError> {
        let space_did = self.space_operator.did();
        Ok(self.user_delegations().await?.iter().any(|d| {
            matches!(
                d.subject(),
                DelegatedSubject::Specific(did) if did == space_did
            )
        }))
    }
}
