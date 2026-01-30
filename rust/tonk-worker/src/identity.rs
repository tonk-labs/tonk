//! User identity management.
//!
//! This module provides the `Identity` type which represents a user's persistent
//! identity on this device. There is exactly one identity per device/browser.

use crate::account::{Account, AccountError};
use crate::key_store::{KeyStore, KeyStoreError};
use crate::session::{Session, SessionError};
use thiserror::Error;
use tonk_space::Operator;
use ucan::did::Ed25519Did;

/// Errors that can occur when working with identity.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// Failed to access key store.
    #[error("Key store error: {0}")]
    KeyStore(#[from] KeyStoreError),

    /// Failed to access account.
    #[error("Account error: {0}")]
    Account(#[from] AccountError),
}

/// A user's persistent identity on this device.
///
/// There is exactly one Identity per device/browser. The identity is backed by
/// an Ed25519 keypair that is generated on first use and persisted to storage.
///
/// The Identity provides access to:
/// - The user's DID (decentralized identifier)
/// - The user's operator (for signing operations)
/// - Methods to open or create sessions
#[derive(Clone)]
pub struct Identity {
    /// The user's operator keypair for this device.
    /// Used to sign operations and establish identity.
    /// TODO: In the future with powerline delegations, this will be the only key
    /// needed - authority over spaces will come from UCAN delegation chains
    /// rather than holding space keys.
    operator: Operator,
    /// Access to the key store for retrieving space operators.
    /// TODO: Once we switch to powerline delegations, space keys won't be stored
    /// and this field may no longer be needed.
    key_store: KeyStore,
    account: Account,
}

impl Identity {
    /// Load an existing identity or create a new one.
    ///
    /// On first call, generates a new random Ed25519 keypair and persists it.
    /// On subsequent calls, loads the existing keypair from storage.
    ///
    /// TODO: Consider passing storage as a parameter to make testing easier
    /// and avoid global state. Should also add a test verifying that loading
    /// across sessions returns the same DID.
    pub async fn load_or_create() -> Result<Self, IdentityError> {
        let key_store = KeyStore::open().await?;

        let operator = match key_store.user_operator().await? {
            Some(op) => op,
            None => key_store.create_user_operator().await?,
        };

        // Prefix with "tonk:" for debug clarity when viewing IndexedDB in devtools
        let db_name = format!("tonk:{}", operator.did());
        let account = Account::open(&db_name, &operator).await?;

        Ok(Self {
            operator,
            key_store,
            account,
        })
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

    /// Get access to the key store.
    pub fn key_store(&self) -> &KeyStore {
        &self.key_store
    }

    /// Get access to the account.
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Get mutable access to the account.
    pub fn account_mut(&mut self) -> &mut Account {
        &mut self.account
    }

    /// Open a session for the given space.
    ///
    /// # Arguments
    /// * `space_did` - The DID of the space to open
    pub async fn open_session(&self, space_did: &str) -> Result<Session, SessionError> {
        Session::open(self, space_did).await
    }

    /// Create a new space owned by this user and open it as a session.
    ///
    /// The new space will have:
    /// - A newly generated Ed25519 keypair
    /// - A delegation granting this user full authority over the space
    pub async fn create_session(&mut self) -> Result<Session, SessionError> {
        Session::create(self).await
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
