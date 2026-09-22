//! Peers and sessions over the CLI's profile.
//!
//! The CLI opens its profile once and hands a `(Profile, Session)` pair
//! around. These helpers build the peer over that already-mounted profile
//! and derive the session the callers used to derive from the profile
//! directly.

use anyhow::{Context as _, Result};
use dialog_capability::Subject;
use dialog_effects::storage::Directory;
use dialog_peer::{Peer, Profile, Session};
use dialog_storage::provider::storage::{NativeSpace, Storage};

/// A peer over `profile`, whose space is already mounted in `storage`,
/// with `base` as the directory its space names resolve against.
pub(crate) async fn peer_for(
    profile: &Profile,
    storage: Storage<NativeSpace>,
    base: Directory,
) -> Result<Peer<NativeSpace>> {
    Peer::new()
        .storage(storage)
        .base(base)
        .attach(profile.signer().clone())
        .await
        .context("failed to build peer")
}

/// A session of `profile`'s peer under `context`, allowed everything the
/// peer holds.
pub(crate) async fn session_for(
    profile: &Profile,
    storage: Storage<NativeSpace>,
    base: Directory,
    context: &[u8],
) -> Result<Session<NativeSpace>> {
    let peer = peer_for(profile, storage, base).await?;
    derive_session(&peer, context).await
}

/// A session of `peer` under `context`, allowed everything the peer holds.
pub(crate) async fn derive_session(
    peer: &Peer<NativeSpace>,
    context: &[u8],
) -> Result<Session<NativeSpace>> {
    let credential = peer
        .derive(context)
        .await
        .context("failed to derive the session key")?;
    peer.session(credential)
        .allow(Subject::any())
        .build()
        .await
        .context("failed to build session")
}
