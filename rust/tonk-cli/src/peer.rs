//! Peers and sessions over the CLI's identity.
//!
//! The CLI opens its peer once and hands `(peer, session)` pairs around.
//! Its data lives in more than one place, so a session is often rebased:
//! the same credential and storage as the peer, with a different base
//! directory for its space names.

use anyhow::{Context as _, Result};
use dialog_capability::Subject;
use dialog_effects::storage::Directory;
use dialog_peer::{Peer, Session};
use dialog_storage::provider::storage::NativeSpace;

/// The CLI's peer: its identity over the native storage.
pub type NativePeer = Peer<NativeSpace>;

/// A session of the CLI's peer.
pub type NativeSession = Session<NativeSpace>;

/// A peer over the same credential and storage as `peer`, with `base` as
/// the directory its space names resolve against.
pub async fn peer_for(peer: &NativePeer, base: Directory) -> Result<NativePeer> {
    Peer::new()
        .storage(peer.storage().clone())
        .base(base)
        .attach(peer.credential().clone())
        .await
        .context("failed to build peer")
}

/// A session of `peer` rebased at `base`, under `context`, allowed
/// everything the peer holds.
pub async fn session_for(
    peer: &NativePeer,
    base: Directory,
    context: &[u8],
) -> Result<NativeSession> {
    derive_session(&peer_for(peer, base).await?, context).await
}

/// A session of `peer` under `context`, allowed everything the peer holds.
pub async fn derive_session(peer: &NativePeer, context: &[u8]) -> Result<NativeSession> {
    peer.derive(context)
        .await
        .context("failed to derive the session key")?
        .allow(Subject::any())
        .build()
        .await
        .context("failed to build session")
}
