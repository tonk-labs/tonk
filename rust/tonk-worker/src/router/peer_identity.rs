//! This device profile's iroh identity, held where the profile holds its
//! other secrets.
//!
//! The endpoint key *is* the peer's name: a remote that stored this
//! identity has to still reach it after a restart, so it cannot be minted
//! per run. It also must not be shared — a device profile is the
//! installation-local identity, and two profiles binding endpoints with
//! one key would be two peers wearing one name, which iroh resolves to
//! one.
//!
//! So it lives in the profile's own credential store: device-local by
//! construction, durable across sessions, and reached through the same
//! `credential().site(..).load()/save()` path a space credential uses. It
//! is deliberately NOT custody — `custody_recipient` seals to the
//! *account's* X25519 recipient, so a seed stored that way would be
//! openable by every device on the account, which is the opposite of what
//! a per-device identity means.

use dialog_capability::SiteId;
use dialog_effects::credential::Secret;

use crate::TonkWorkerError;

/// The credential-store key this profile's peer seed is held under.
///
/// A fixed name rather than one derived from the profile DID: the store
/// is already scoped to the profile, so keying it again would only add a
/// way for the two to disagree.
const PEER_SEED_SITE: &str = "tonk:peer-seed";

/// The seed, loading it or minting one the first time.
///
/// Lookup-or-generate rather than generate-and-hope: the first call on a
/// device mints and saves, every later call returns the same bytes, and a
/// remote that recorded this peer keeps reaching it.
pub async fn seed(tonk: &crate::worker::TonkState) -> Result<[u8; 32], TonkWorkerError> {
    if let Some(found) = load(tonk).await? {
        return Ok(found);
    }

    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|error| {
        TonkWorkerError::Internal(format!("could not generate a peer seed: {error}"))
    })?;

    tonk.profile
        .credential()
        .site(SiteId::from(PEER_SEED_SITE.to_owned()))
        .save(Secret::from(seed.to_vec()))
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("could not store the peer seed: {error}"))
        })?;

    Ok(seed)
}

/// The stored seed, or `None` on a profile that has never had one.
///
/// A stored value of the wrong length is treated as absent rather than as
/// an error: the only way to produce one is a different version of this
/// code, and refusing to start is a worse answer than minting a fresh
/// identity.
async fn load(tonk: &crate::worker::TonkState) -> Result<Option<[u8; 32]>, TonkWorkerError> {
    let stored = tonk
        .profile
        .credential()
        .site(SiteId::from(PEER_SEED_SITE.to_owned()))
        .load::<Secret>()
        .perform(&tonk.operator)
        .await;

    match stored {
        Ok(secret) => Ok(<[u8; 32]>::try_from(secret.as_bytes()).ok()),
        // Absent is the ordinary first state, not a failure.
        Err(_) => Ok(None),
    }
}
