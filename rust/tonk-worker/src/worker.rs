//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    Identity, Session, api_router,
    axum::{RequestConversion, ResponseConversion},
    session::SessionError,
};
use axum::{Router, body::Body};
use js_sys::Promise;
use tokio::sync::{Mutex, RwLock};
use tonk_common::log;
use tonk_space::Operator;
use tower_service::Service;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{Request, Response};

/// Application state containing the user's identity and session cache.
pub struct TonkState {
    /// The user's persistent identity.
    pub identity: Arc<RwLock<Identity>>,
    /// Cache of active sessions by space DID.
    /// Sessions are lazily loaded when a space is first accessed.
    pub sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl TonkState {
    /// Convert a multikey (z6Mk...) to a full DID (did:key:z6Mk...).
    pub fn multikey_to_did(multikey: &str) -> String {
        if multikey.starts_with("did:key:") {
            multikey.to_string()
        } else {
            format!("did:key:{}", multikey)
        }
    }

    /// Get or create a session for the given space.
    ///
    /// If the session is already cached, returns a clone.
    /// Otherwise, opens a new session and caches it.
    pub async fn session_for_space(&self, space_did: &str) -> Result<Session, SessionError> {
        // Check cache first
        {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(space_did) {
                return Ok(session.clone());
            }
        }

        // Not cached, create new session
        log!("Opening session for space: {}", space_did);
        let identity = self.identity.read().await;
        let session = identity.open_session(space_did).await?;

        // Cache it
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(space_did.to_string(), session.clone());
        }

        Ok(session)
    }

    /// Update a cached session after modification.
    ///
    /// This should be called after mutating a session to ensure the cache
    /// reflects the current state.
    pub async fn update_session(&self, session: Session) {
        let space_did = session.space_did().to_string();
        let mut sessions = self.sessions.write().await;
        sessions.insert(space_did, session);
    }
}

/// The main Tonk service worker that handles browser fetch events.
///
/// This struct bridges the browser's service worker API with an Axum router,
/// allowing HTTP-like request handling in a Wasm context.
#[wasm_bindgen]
pub struct TonkServiceWorker {
    router: Arc<Mutex<Router>>,
}

#[wasm_bindgen]
impl TonkServiceWorker {
    /// Creates a new service worker instance.
    ///
    /// Initializes the user identity, session, and API router.
    ///
    /// On first run:
    /// - Creates a new random identity for the user
    /// - Creates a new space with a delegation granting the user ownership
    ///
    /// On subsequent runs:
    /// - Loads the existing identity from IndexedDB
    /// - Opens the first known space (or creates one if none exist)
    ///
    /// The worker creates two keypairs:
    /// - **Space keypair**: Represents the space identity
    /// - **Operator keypair**: Used for signing operations
    ///
    /// A delegation is created from the space to the operator, granting
    /// full capabilities. This delegation is used when setting up upstream sync.
    ///
    /// # Errors
    ///
    /// Returns a `JsError` if the service worker cannot be initialized.
    pub async fn new() -> Result<Self, JsError> {
        log!("Tonk worker initializing...");

        // 1. Load or create user identity
        let mut identity = Identity::load_or_create()
            .await
            .expect_throw("Could not initialize identity");
        log!("User DID: {}", identity.did());

        // 2. Ensure at least one space exists (the shared space)
        let known_spaces = identity
            .account()
            .known_spaces()
            .await
            .expect_throw("Could not query known spaces");

        if known_spaces.is_empty() {
            log!("No known spaces, joining shared space");
            let shared_space = Operator::from_passphrase("public tonk space").await;
            identity
                .join_session(shared_space)
                .await
                .expect_throw("Could not join shared space");
        }

        // 3. Build state with empty session cache (sessions loaded on demand)
        let state = TonkState {
            identity: Arc::new(RwLock::new(identity)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        };
        let router = Arc::new(Mutex::new(api_router(state)));

        Ok(Self { router })
    }

    /// Handles incoming fetch events from the browser.
    ///
    /// Converts the browser's `Request` to an Axum request, processes it through
    /// the router, and converts the response back to a browser `Response`.
    ///
    /// # Parameters
    ///
    /// - `request`: The incoming browser fetch request
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to the response
    #[wasm_bindgen(js_name = "onfetch")]
    pub fn on_fetch(&self, request: Request) -> Promise {
        log!("Handling fetch!");

        let router = self.router.clone();
        let request: axum::http::Request<Body> =
            RequestConversion::from(request).try_into().unwrap_throw();

        future_to_promise(async move {
            let response = router
                .lock()
                .await
                .call(request)
                .await
                .expect_throw("Failed to handle API request");
            ResponseConversion::from(response)
                .try_into()
                .map(|value: Response| JsValue::from(value))
                .map_err(JsValue::from)
        })
    }
}
