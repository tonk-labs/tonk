//! Service-worker client identity, and the guest bindings keyed by it.

/// A service worker Client ID, extracted from a `FetchEvent` by
/// the worker's `on_fetch` and attached to each request as an
/// extension.
///
/// This is the stable identifier for the document/worker that
/// initiated the request — it outlives any single fetch and is
/// stable for the lifetime of the document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);

/// Binding that records which repository and branch a guest client is
/// bound to. Nothing registers one any more: the iframe-navigation route
/// that did was retired once portal `srcdoc` guests replaced it, so the
/// map stays empty and the checks that consult it pass through.
#[derive(Clone, Debug)]
pub struct ViewBinding {
    /// The repository name the iframe is scoped to.
    pub repo: String,
    /// The branch name the iframe is scoped to.
    pub branch: String,
}

/// Shared map of `ClientId → ViewBinding`. Lives on
/// `TonkState::view_bindings` (renamed from `guests`).
pub type ViewBindings =
    std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<ClientId, ViewBinding>>>;
