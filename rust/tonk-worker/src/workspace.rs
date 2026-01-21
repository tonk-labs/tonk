//! Workspace - an opened space with user context.
//!
//! A Workspace represents an active session where a user is working with a specific
//! space. It combines the space data with the user's identity context, enabling
//! operations that require both (like querying delegations granted to the user).

use std::sync::Arc;

use thiserror::Error;
use tonk_space::{Delegation, Operator, Space, SpaceError};
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::{Ed25519Did, Ed25519Signer};

use crate::ServiceWorkerStorageBackend;
use crate::identity::Identity;
use crate::user_store::UserStoreError;

/// Errors that can occur when working with workspaces.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// No default space has been set.
    #[error("No default space configured")]
    NoDefaultSpace,

    /// The requested space was not found.
    #[error("Space not found: {0}")]
    SpaceNotFound(String),

    /// Failed to access user store.
    #[error("User store error: {0}")]
    UserStore(#[from] UserStoreError),

    /// Failed to operate on space.
    #[error("Space error: {0}")]
    Space(#[from] SpaceError),

    /// Failed to create delegation.
    #[error("Delegation error: {0}")]
    Delegation(String),
}

/// An active workspace - a space opened by a specific user.
///
/// Combines the space, the user's view of it, and operations requiring both.
/// The workspace holds a reference to the user's identity, allowing it to
/// perform operations that need both user and space context.
pub struct Workspace {
    /// Reference to the user's identity.
    identity: Arc<Identity>,
    /// The opened space.
    space: Space<ServiceWorkerStorageBackend>,
    /// The space's own keypair (for signing space-level operations).
    space_operator: Operator,
}

impl Workspace {
    /// Open an existing space (or the default space if space_did is None).
    ///
    /// # Arguments
    /// * `identity` - The user's identity
    /// * `space_did` - The DID of the space to open, or None for the default space
    ///
    /// # Errors
    /// - `WorkspaceError::NoDefaultSpace` if space_did is None and no default is set
    /// - `WorkspaceError::SpaceNotFound` if the space secret is not stored
    pub(crate) async fn open(
        identity: &Identity,
        space_did: Option<&str>,
    ) -> Result<Self, WorkspaceError> {
        let space_did = match space_did {
            Some(did) => did.to_string(),
            None => identity
                .store()
                .get_default_space()
                .await?
                .ok_or(WorkspaceError::NoDefaultSpace)?,
        };

        let secret = identity
            .store()
            .get_space_secret(&space_did)
            .await?
            .ok_or_else(|| WorkspaceError::SpaceNotFound(space_did.clone()))?;

        let space_operator = Operator::from_secret(secret);

        // Open the space database
        let db_name = format!("tonk-space:{}", space_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;
        let space = Space::open(space_did, &space_operator, backend).await?;

        Ok(Self {
            identity: Arc::new(identity.clone()),
            space,
            space_operator,
        })
    }

    /// Create a new space owned by this user.
    ///
    /// This will:
    /// 1. Generate a new Ed25519 keypair for the space
    /// 2. Create a delegation from the space to the user (granting full authority)
    /// 3. Store the space secret in the user store
    /// 4. Set this space as the default space
    /// 5. Create the space with the ownership delegation
    pub(crate) async fn create(identity: &Identity) -> Result<Self, WorkspaceError> {
        // Generate a new keypair for the space
        let space_operator = Operator::generate();
        let space_did = space_operator.did().to_string();

        // Create delegation: space -> user (space grants user full authority)
        let delegation = Self::create_ownership_delegation(&space_operator, identity.operator())?;

        // Clone identity and get mutable store access
        let mut identity_clone = identity.clone();
        let store = identity_clone.store_mut();

        // Store space secret in user store
        store
            .set_space_secret(&space_did, space_operator.to_secret())
            .await?;

        // Set as default space
        store.set_default_space(&space_did).await?;

        // Create the space database and space with ownership delegation
        let db_name = format!("tonk-space:{}", space_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;
        let space = Space::create(space_did, &space_operator, backend, vec![delegation]).await?;

        Ok(Self {
            identity: Arc::new(identity.clone()),
            space,
            space_operator,
        })
    }

    /// Create an ownership delegation from space to user.
    ///
    /// The delegation grants the user full authority (`/*`) over the space.
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
    pub fn user_did(&self) -> &Ed25519Did {
        self.identity.did()
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
    ///
    /// This queries the space for delegations where the audience matches
    /// the current user's DID.
    pub async fn user_delegations(&self) -> Vec<Delegation> {
        self.space
            .delegations_for_audience(&self.identity.did().to_string())
            .await
    }

    /// Check if the current user owns this space.
    ///
    /// A user owns a space if they have a delegation where:
    /// - The audience is the user's DID
    /// - The subject is the space's DID
    pub async fn user_is_owner(&self) -> bool {
        let space_did = self.space_operator.did();
        self.user_delegations().await.iter().any(|d| {
            matches!(
                d.subject(),
                DelegatedSubject::Specific(did) if did == space_did
            )
        })
    }
}

#[cfg(test)]
mod tests {
    // Tests require a mock or the actual IndexedDB backend, which is WASM-only.
    // Integration tests should be run in the browser context.
}
