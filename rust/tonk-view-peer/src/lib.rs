//! A shown view's peer, run in a Web Worker beside an MCP Apps view
//! (plan #1200 stage 7, route B).
//!
//! The worker makes the view's key ([`ViewKey::generate`], WebCrypto,
//! non-extractable), the relay asks the host for `tonk_view_access` with
//! its DID, and [`ViewPeer::open`] opens the ordinary router over that key
//! and the delegations, with the space mounted as a replica of the
//! provider. The guest's requests reach it through [`ViewPeer::fetch`],
//! the same conversion the service worker's fetch handler uses.

use std::rc::Rc;

use dialog_credentials::Ed25519Signer;
use dialog_effects::storage::Directory;
use dialog_repository::HeldCaches;
use dialog_ucan_core::{Container, DelegationChain};
use dialog_varsig::{Did, Principal as _};
use js_sys::Promise;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use tonk_worker::HostedPeer;
use tonk_worker::axum::{RequestConversion, ResponseConversion};

/// A view's key: generated in this worker, never exported.
#[wasm_bindgen]
pub struct ViewKey {
    signer: Ed25519Signer,
}

#[wasm_bindgen]
impl ViewKey {
    /// Make a new non-extractable key.
    pub async fn generate() -> Result<ViewKey, JsError> {
        let signer = Ed25519Signer::generate()
            .await
            .map_err(|error| JsError::new(&error.to_string()))?;
        Ok(Self { signer })
    }

    /// The key's `did:key`, the audience a view's delegation names.
    #[wasm_bindgen(getter)]
    pub fn did(&self) -> String {
        self.signer.did().to_string()
    }
}

/// A view's peer over one space.
#[wasm_bindgen]
pub struct ViewPeer {
    peer: Rc<HostedPeer>,
}

#[wasm_bindgen]
impl ViewPeer {
    /// Open a peer acting as `key` with `delegations` (base64url
    /// containers, as `tonk_view_access` returns them), with `space`
    /// mounted as a replica of `remote` (the provider's `/ucan/`), its
    /// storage under `directory`, which nothing else may use.
    pub async fn open(
        key: &ViewKey,
        delegations: Vec<String>,
        space: String,
        remote: String,
        directory: String,
    ) -> Result<ViewPeer, JsError> {
        let error = |error: &dyn std::fmt::Display| JsError::new(&error.to_string());
        let proofs = delegations
            .iter()
            .map(|encoded| {
                let container =
                    Container::decode(encoded.trim().as_bytes()).map_err(|e| error(&e))?;
                DelegationChain::try_from(container).map_err(|e| error(&e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let space: Did = space.parse().map_err(|e| error(&e))?;
        let remote = url::Url::parse(&remote).map_err(|e| error(&e))?;
        let peer = HostedPeer::open_as(
            Directory::At(directory),
            key.signer.clone(),
            &proofs,
            HeldCaches::new(),
        )
        .await
        .map_err(|e| error(&e))?;
        peer.mount(&space, &remote).await.map_err(|e| error(&e))?;
        Ok(Self {
            peer: Rc::new(peer),
        })
    }

    /// Answer a request on the ordinary router, as the service worker
    /// answers one. A subscription's response stays a stream.
    pub fn fetch(&self, request: web_sys::Request) -> Promise {
        let peer = self.peer.clone();
        future_to_promise(async move {
            let request = RequestConversion::from(request)
                .into_axum_request()
                .await
                .map_err(JsValue::from)?;
            let method = request.method().clone();
            let response = peer.call(request).await;
            web_sys::Response::try_from(ResponseConversion::new(method, response))
                .map(JsValue::from)
                .map_err(JsValue::from)
        })
    }

    /// The DID the peer acts as.
    pub async fn did(&self) -> String {
        self.peer.did().await.to_string()
    }
}
