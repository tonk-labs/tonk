//! A peer built for one request and dropped after it: the ordinary worker
//! over in-memory storage, acting with a key and delegations the caller
//! hands in. Nothing is kept between requests; each opens its spaces as
//! replicas of their remote and pulls before it reads.
//!
//! Built for the hosted agent peer spike (a Cloudflare Worker), where the
//! key comes from a session token and the authority from its powerline.
use std::sync::Arc;

use axum::{Router, body::Body, http::Request, response::Response};
use dialog_capability::{Subject, did};
use dialog_credentials::{Credential, Ed25519Signer, SignerCredential};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal as _};
use tower_service::Service;

use super::{AppState, LspHub};
use crate::TonkWorkerError;

/// The ordinary worker router over a peer that exists for one request.
pub struct HostedPeer {
    router: Router,
    state: AppState,
    hub: Arc<LspHub>,
    directory: Directory,
}

impl HostedPeer {
    /// Open a peer acting with the key `seed` derives, holding `proofs`.
    ///
    /// `name` scopes the in-memory directory; two peers open at once need
    /// different names.
    pub async fn open(
        name: &str,
        seed: &[u8; 32],
        proofs: &[DelegationChain],
    ) -> Result<Self, TonkWorkerError> {
        let directory = Directory::At(format!("/hosted/{name}"));
        let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
        let (credentials, _) = crate::device::open_system(directory.clone())
            .await
            .map_err(|error| internal(&error))?;
        let signer = Ed25519Signer::import(seed)
            .await
            .map_err(|error| internal(&error))?;
        let did = signer.did();
        Subject::from(did!("local:storage"))
            .attenuate(storage_fx::Storage)
            .attenuate(Location::new(directory.clone(), "hosted"))
            .create(Credential::Signer(SignerCredential::from(signer)))
            .perform(&credentials)
            .await
            .map_err(|error| internal(&error))?;
        let registry = crate::device::Registry::new("hosted".to_owned(), directory.clone());
        let storage = registry.storage().await?;
        let profile = registry.open_profile(&storage, "hosted").await?;
        let state =
            crate::worker::boot_state(storage, "hosted".to_owned(), profile, registry).await?;
        if state.profile.did() != did {
            return Err(TonkWorkerError::Internal("hosted identity mismatch".into()));
        }
        for chain in proofs {
            state
                .profile
                .access()
                .save(UcanDelegation(chain.clone()))
                .perform(&state.operator)
                .await
                .map_err(|error| internal(&error))?;
        }
        let (router, state, hub) = super::api_router_with_state(state);
        Ok(Self {
            router,
            state,
            hub,
            directory,
        })
    }

    /// Mount `subject` as a replica of the space served at `remote`. Its
    /// contents arrive with the next pull.
    pub async fn mount(&self, subject: &Did, remote: &url::Url) -> Result<(), TonkWorkerError> {
        let state = self.state.read().await;
        super::join::mount_replica(&state, subject, Some(remote.as_str()), None).await?;
        super::repository::record_initialized_replica_in_profile(&state, subject)
            .await
            .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
        Ok(())
    }

    /// The DID this peer signs as.
    pub async fn did(&self) -> Did {
        self.state.read().await.profile.did()
    }

    /// Serve one request with the ordinary worker router.
    pub async fn call(&mut self, request: Request<Body>) -> Response {
        match self.router.call(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }

    /// Release the peer's subscriptions and its in-memory keys.
    pub async fn close(self) {
        self.state.read().await.retire();
        self.hub.shutdown().await;
        crate::device::forget_system(&self.directory);
    }
}
