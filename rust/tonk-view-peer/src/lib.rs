//! A shown view's peer, run in a Web Worker beside an MCP Apps view
//! (plan #1200 stage 7, route B).
//!
//! The worker makes the view's key ([`ViewKey::generate`], WebCrypto,
//! non-extractable), the relay asks the host for `tonk_view_access` with
//! its DID, and [`ViewPeer::open`] boots the space's own kernel acting as
//! that key, holding the delegations, with the space kept in this origin's
//! IndexedDB. The guest's requests reach the kernel through
//! [`ViewPeer::fetch`], as a space's service worker hands them to its
//! kernel, and the kernel syncs the space while a view watches it.

use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::{Container, DelegationChain};
use dialog_varsig::Principal as _;
use js_sys::{Array, Function, Promise, Uint8Array};
use serde_json::json;
use wasm_bindgen::prelude::*;

use tonk_kernel_host::embedder::{Embedder, boot_handed};

/// The origin a view's requests name, which the kernel serves.
const ORIGIN: (&str, &str) = ("https", "view.invalid");

/// The header a space's kernel learns the page a request came from by.
const CLIENT_HEADER: &str = "x-tonk-client-id";

/// A view's key: generated in this worker, non-extractable. It leaves the
/// worker only as opaque `CryptoKey` handles, which the relay keeps in
/// IndexedDB so a view opened again signs as the same key and reads the
/// store it pulled before.
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

    /// The key's `CryptoKey` handles, to keep in IndexedDB. They carry no
    /// key material.
    pub async fn export(&self) -> Result<JsValue, JsError> {
        let export = self
            .signer
            .export()
            .await
            .map_err(|error| JsError::new(&error.to_string()))?;
        Ok(export.into())
    }

    /// The key [`export`](Self::export) handed out.
    pub async fn restore(handles: JsValue) -> Result<ViewKey, JsError> {
        let export = dialog_credentials::key::KeyExport::try_from(handles)
            .map_err(|error| JsError::new(&error.to_string()))?;
        let signer = Ed25519Signer::import(export)
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

/// A view's peer over one space: the space's kernel.
#[wasm_bindgen]
pub struct ViewPeer {
    kernel: Embedder,
    /// The space the peer shows.
    space: String,
}

#[wasm_bindgen]
impl ViewPeer {
    /// Boot `kernel`, the space's kernel component, over `space`, acting as
    /// `key` with `delegations` (base64url containers, as
    /// `tonk_view_access` returns them), its storage under `directory`,
    /// which nothing else may use. The kernel takes the first delegation up,
    /// which says where the space syncs, and keeps the rest.
    pub async fn open(
        key: &ViewKey,
        delegations: Vec<String>,
        space: String,
        kernel: Uint8Array,
        directory: String,
    ) -> Result<ViewPeer, JsError> {
        let error = |error: &dyn std::fmt::Display| JsError::new(&error.to_string());
        let (first, rest) = delegations
            .split_first()
            .ok_or_else(|| JsError::new("the view holds no delegation"))?;
        let chain = decode(first)?.to_bytes().map_err(|e| error(&e))?;
        let kernel = boot_handed(
            &kernel.to_vec(),
            space.clone(),
            directory,
            (ORIGIN.0.to_owned(), ORIGIN.1.to_owned()),
            key.signer.clone(),
            chain,
            Function::new_no_args(""),
        )
        .await?;
        let peer = Self { kernel, space };
        peer.keep(rest).await?;
        Ok(peer)
    }

    /// Pull the space: another view's peer over the same store committed or
    /// pulled.
    pub async fn refresh(&self) -> Result<(), JsError> {
        let path = format!("/api/repository/{}/branch/main/sync/pull", self.space);
        self.ask("POST", &path, "text/plain", b"").await
    }

    /// Hold `delegations` too: a renewed one, or another view's.
    pub async fn grant(&self, delegations: Vec<String>) -> Result<(), JsError> {
        self.keep(&delegations).await
    }

    /// Hand the kernel a request of the guest document `client` (each view
    /// of a peer shared by several names its own): `method` `path`, with
    /// `headers` as `[name, value]` pairs and `body`. Answers `{ status,
    /// headers, body }` once the kernel has its status, with the body read
    /// piece by piece as the kernel writes it, so a subscription stays a
    /// stream.
    pub fn fetch(
        &self,
        method: String,
        path: String,
        headers: Array,
        body: Uint8Array,
        client: Option<String>,
    ) -> Promise {
        if let Some(client) = client {
            headers.push(&Array::of2(&CLIENT_HEADER.into(), &client.into()));
        }
        self.kernel.handle(method, path, headers, body)
    }
}

impl ViewPeer {
    /// Have the kernel keep each of `delegations`, checked as a take-up
    /// checks its own.
    async fn keep(&self, delegations: &[String]) -> Result<(), JsError> {
        if delegations.is_empty() {
            return Ok(());
        }
        let path = format!("/api/repository/{}/delegations", self.space);
        let body = json!({ "chains": delegations }).to_string();
        self.ask("POST", &path, "application/json", body.as_bytes())
            .await
    }

    async fn ask(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<(), JsError> {
        let headers = vec![("content-type".to_owned(), content_type.as_bytes().to_vec())];
        let (status, answer) = self
            .kernel
            .answer(method, path, headers, body)
            .await
            .map_err(|error| JsError::new(&error))?;
        if (200..300).contains(&status) {
            return Ok(());
        }
        Err(JsError::new(&format!(
            "{method} {path} answered {status}: {}",
            String::from_utf8_lossy(&answer)
        )))
    }
}

/// A delegation as `tonk_view_access` returns one: a base64url container.
fn decode(encoded: &str) -> Result<DelegationChain, JsError> {
    let error = |error: &dyn std::fmt::Display| JsError::new(&error.to_string());
    let container = Container::decode(encoded.trim().as_bytes()).map_err(|e| error(&e))?;
    DelegationChain::try_from(container).map_err(|e| error(&e))
}
