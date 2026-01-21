//! User identity management.
//!
//! This module provides the `Identity` type which represents a user's persistent
//! identity on this device. There is exactly one identity per device/browser.

use crate::user_store::{UserStore, UserStoreError};
use crate::workspace::{Workspace, WorkspaceError};
use thiserror::Error;
use tonk_space::Operator;
use ucan::did::Ed25519Did;

/// Errors that can occur when working with identity.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// Failed to access user store.
    #[error("User store error: {0}")]
    Store(#[from] UserStoreError),
}

/// A user's persistent identity on this device.
///
/// There is exactly one Identity per device/browser. The identity is backed by
/// an Ed25519 keypair that is generated on first use and persisted to IndexedDB.
///
/// The Identity provides access to:
/// - The user's DID (decentralized identifier)
/// - The user's operator (for signing operations)
/// - Methods to open or create workspaces
#[derive(Clone)]
pub struct Identity {
    operator: Operator,
    store: UserStore,
}

impl Identity {
    /// Load an existing identity or create a new one.
    ///
    /// On first call, generates a new random Ed25519 keypair and persists it.
    /// On subsequent calls, loads the existing keypair from storage.
    pub async fn load_or_create() -> Result<Self, IdentityError> {
        let mut store = UserStore::open().await?;

        let operator = match store.get_identity_secret().await? {
            Some(secret) => Operator::from_secret(secret),
            None => {
                let operator = Operator::generate();
                store.set_identity_secret(operator.to_secret()).await?;
                operator
            }
        };

        Ok(Self { operator, store })
    }

    /// Get the DID (decentralized identifier) for this identity.
    ///
    /// The DID is derived from the public key and has the format `did:key:z6Mk...`.
    pub fn did(&self) -> &Ed25519Did {
        self.operator.did()
    }

    /// Get the operator for signing operations.
    pub fn operator(&self) -> &Operator {
        &self.operator
    }

    /// Open a workspace for the given space, or the default space if None.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space to open, or None for the default space
    ///
    /// # Errors
    /// Returns `WorkspaceError::NoDefaultSpace` if no space_did is provided and
    /// no default space has been set.
    pub async fn open_workspace(&self, space_did: Option<&str>) -> Result<Workspace, WorkspaceError> {
        Workspace::open(self, space_did).await
    }

    /// Create a new space owned by this user and open it as a workspace.
    ///
    /// The new space will have:
    /// - A newly generated Ed25519 keypair
    /// - A delegation granting this user full authority over the space
    /// - The space set as the default space for this user
    pub async fn create_workspace(&self) -> Result<Workspace, WorkspaceError> {
        Workspace::create(self).await
    }

    /// Get access to the user store (internal use by Workspace).
    pub(crate) fn store(&self) -> &UserStore {
        &self.store
    }

    /// Get mutable access to the user store (internal use by Workspace).
    pub(crate) fn store_mut(&mut self) -> &mut UserStore {
        &mut self.store
    }
}

impl std::fmt::Display for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.did())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_creates_identity_with_valid_did() {
        let identity = Identity::load_or_create().await.unwrap();
        let did = identity.did().to_string();
        assert!(did.starts_with("did:key:z"));
    }

    #[tokio::test]
    async fn it_displays_as_did() {
        let identity = Identity::load_or_create().await.unwrap();
        let display = format!("{}", identity);
        assert!(display.starts_with("did:key:z"));
    }
}
