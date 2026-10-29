//! A peer built for one request and dropped after it: the ordinary worker
//! over in-memory storage, acting with a key and delegations the caller
//! hands in. Nothing is kept between requests; each opens its spaces as
//! replicas of their remote and pulls before it reads.
//!
//! Built for the hosted agent peer spike (a Cloudflare Worker), where the
//! key comes from a session token and the authority from its powerline.
//! [`SpaceWriter`] keeps one open for a space and an MCP session and
//! serializes its writes.
use std::sync::Arc;

use axum::{Router, body::Body, http::Request, response::Response};
use dialog_capability::{Subject, did};
use dialog_credentials::{Credential, Ed25519Signer, SignerCredential};
use dialog_effects::storage::{self as storage_fx, Directory, Location, LocationExt};
use dialog_repository::HeldCaches;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal as _};
use tower_service::Service;

use super::{AppState, LspHub};
use crate::TonkWorkerError;

pub mod access;
pub mod mcp;
pub mod view;
mod writer;
pub use writer::{NODE_CACHE_BUDGET, SpaceWriter, WriterKey, WriterSession, writer_seed};

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
        Self::open_in(
            Directory::At(format!("/hosted/{name}")),
            seed,
            proofs,
            HeldCaches::new(),
        )
        .await
    }

    /// [`open`](Self::open) in `directory`, its repositories holding
    /// `caches`. Two peers open at once need different directories.
    pub async fn open_in(
        directory: Directory,
        seed: &[u8; 32],
        proofs: &[DelegationChain],
        caches: HeldCaches,
    ) -> Result<Self, TonkWorkerError> {
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
        let registry =
            crate::device::Registry::holding("hosted".to_owned(), directory.clone(), caches);
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

    /// Point this peer's access branch at the agent space `agent`, served
    /// at `home`, and pull it: the grants retained there become proofs
    /// this peer's operator finds. See [`access::adopt_agent_access`].
    pub async fn adopt_access(&self, agent: &Did, home: &url::Url) -> Result<(), TonkWorkerError> {
        let state = self.state.read().await;
        access::adopt_agent_access(&state, agent, home).await
    }

    /// The spaces the agent space `agent` reaches, read from its replica
    /// here as it is now. See [`access::read_agent_access`].
    pub async fn agent_access(&self, agent: &Did) -> Result<access::AgentAccess, TonkWorkerError> {
        let state = self.state.read().await;
        access::read_agent_access(&state, agent).await
    }

    /// The DID this peer signs as.
    pub async fn did(&self) -> Did {
        self.state.read().await.profile.did()
    }

    /// The DID this peer's revisions are issued by: the worker's session
    /// operator, derived from [`did`](Self::did).
    pub async fn operator(&self) -> Did {
        self.state.read().await.operator.did()
    }

    /// Serve one request with the ordinary worker router. Requests may
    /// overlap: each is served by its own handle on the router.
    pub async fn call(&self, request: Request<Body>) -> Response {
        match self.router.clone().call(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }

    /// Release the peer's subscriptions and its in-memory keys.
    pub async fn close(self) {
        self.shut_down().await;
    }

    /// [`close`](Self::close), for a peer something else may still hold:
    /// requests it serves from here on fail.
    pub async fn shut_down(&self) {
        self.state.read().await.retire();
        self.hub.shutdown().await;
        crate::device::forget_system(&self.directory);
    }
}
