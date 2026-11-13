//! The service-worker client a request came from.

/// A service worker Client ID, extracted from a `FetchEvent` by
/// the worker's `on_fetch` and attached to each request as an
/// extension.
///
/// This is the stable identifier for the document/worker that
/// initiated the request — it outlives any single fetch and is
/// stable for the lifetime of the document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);
