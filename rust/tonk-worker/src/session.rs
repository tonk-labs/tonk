//! Session - an active authorization context for working with a space.
//!
//! A Session represents an operator authorized to act on behalf of an account
//! in a specific space. It combines the space data with the user's identity
//! context, enabling operations that require both (like querying delegations
//! granted to the user).

use thiserror::Error;
use tonk_space::{Delegation, Operator, SpaceError};
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::Ed25519Signer;

use crate::ServiceWorkerStorageBackend;
use crate::account::AccountError;
use crate::identity::Identity;
use crate::key_store::KeyStoreError;
use tonk_space::Space;

/// Errors that can occur when working with sessions.
#[derive(Debug, Error)]
pub enum SessionError {
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

/// An active session - an operator authorized to act on behalf of an account in a space.
pub struct Session {
    /// The account's DID (the identity that authorized this session).
    account: String,
    /// The opened space.
    space: Space<ServiceWorkerStorageBackend>,
    /// The operator for signing operations in this space.
    /// TODO: Once we switch to powerline delegations, this will be the user's
    /// operator and authority will come from delegation chains rather than
    /// holding the space's secret key.
    operator: Operator,
}

impl Session {
    /// Open a session for an existing space.
    ///
    /// # Arguments
    /// * `identity` - The user's identity
    /// * `space_did` - The DID of the space to open
    pub(crate) async fn open(identity: &Identity, space_did: &str) -> Result<Self, SessionError> {
        // Get the space operator from key store
        // TODO: Once we switch to powerline delegations, we won't need to look up
        // space keys - authority will come from delegation chains instead.
        let operator = identity
            .key_store()
            .space_operator(space_did)
            .await?
            .ok_or_else(|| SessionError::SpaceNotFound(space_did.to_string()))?;

        // Open the space database using the user's operator as the replica issuer.
        // This ensures that when making remote requests, the claim.audience() matches
        // the delegation.audience() (which is the user's operator DID).
        // Prefix with "tonk:" for debug clarity when viewing IndexedDB in devtools
        let db_name = format!("tonk:{}", space_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;
        let space = Space::open(space_did.to_string(), identity.operator(), backend).await?;

        Ok(Self {
            account: identity.did().to_string(),
            space,
            operator,
        })
    }

    /// Create a new space owned by this user.
    ///
    /// TODO: Decouple creation from saving - consider having a separate
    /// `.save(&mut storage)` method for persistence rather than doing it implicitly.
    pub(crate) async fn create(identity: &mut Identity) -> Result<Self, SessionError> {
        // Generate new keypair for the space and import it
        Self::import(identity, Operator::generate()).await
    }

    /// Imports a space by making this user an owner.
    pub(crate) async fn import(
        identity: &mut Identity,
        space_operator: Operator,
    ) -> Result<Self, SessionError> {
        // Store space operator in key store
        identity
            .key_store()
            .store_space_operator(&space_operator)
            .await?;

        // Create delegation: space -> user (space grants user full authority)
        let delegation =
            Self::create_ownership_delegation(&space_operator, identity.operator()).await?;

        // Update account with the new space
        let space_did = space_operator.did().to_string();
        identity.account_mut().add_known_space(&space_did).await?;

        // Create the space database using the user's operator as the replica issuer.
        // This ensures that when making remote requests, the claim.audience() matches
        // the delegation.audience() (which is the user's operator DID).
        // Prefix with "tonk:" for debug clarity when viewing IndexedDB in devtools
        let db_name = format!("tonk:{}", space_did);
        let backend = ServiceWorkerStorageBackend::new(&db_name).await;
        let space =
            Space::create(space_did, identity.operator(), backend, vec![delegation]).await?;

        Ok(Self {
            account: identity.did().to_string(),
            space,
            operator: space_operator,
        })
    }

    /// Create an ownership delegation from space to user.
    async fn create_ownership_delegation(
        space_operator: &Operator,
        user_operator: &Operator,
    ) -> Result<Delegation, SessionError> {
        let signer = Ed25519Signer::from(space_operator);
        let ucan_delegation = Delegation::builder()
            .issuer(signer)
            .audience(user_operator.did().clone())
            .subject(DelegatedSubject::Specific(space_operator.did().clone()))
            .command(vec![]) // Empty command = "/*" (all commands)
            .try_build()
            .await
            .expect("Delegation builder should not fail with valid inputs");

        Ok(Delegation::from(ucan_delegation))
    }

    // === Accessors ===

    /// Get the account's DID.
    pub fn account(&self) -> &str {
        &self.account
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

    /// Get the operator (for signing operations in this space).
    pub fn operator(&self) -> &Operator {
        &self.operator
    }

    // === Operations requiring both account and space context ===

    /// Get all delegations granted to this account for this space.
    pub async fn account_delegations(&self) -> Result<Vec<Delegation>, SessionError> {
        Ok(self.space.delegations_for_audience(&self.account).await?)
    }

    /// Check if the current account owns this space.
    pub async fn account_is_owner(&self) -> Result<bool, SessionError> {
        let space_did = self.operator.did();
        Ok(self.account_delegations().await?.iter().any(|d| {
            matches!(
                d.subject(),
                DelegatedSubject::Specific(did) if did == space_did
            )
        }))
    }
}
