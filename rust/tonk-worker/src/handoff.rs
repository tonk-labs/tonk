//! Carry the session overlay from an outgoing service worker to its successor.
//!
//! The overlay lives only in the worker's memory, so a replacement worker
//! would start without it. On retirement the outgoing worker writes a
//! snapshot of every cached branch's overlay to CacheStorage, even an empty
//! one, so the successor can tell "nothing to carry" from "not written yet".
//! The successor's JS `activate` handler holds activation until the snapshot
//! exists (bounded), and the successor takes it once while booting, before
//! it serves a request, and restores it. A snapshot older than
//! [`MAX_AGE_MS`] is discarded instead: after that long its facts may no
//! longer describe the session.

use dialog_reactor::OverlaySnapshot;
use serde::{Deserialize, Serialize};

/// How long a snapshot stays trustworthy. A replacement lands milliseconds
/// after retirement, so anything older belongs to a handoff that never
/// completed.
pub(crate) const MAX_AGE_MS: u64 = 60_000;

/// The snapshot as stored: when it was taken and what it holds.
#[derive(Debug, Serialize, Deserialize)]
struct Handoff {
    exported_at: u64,
    overlays: Vec<OverlaySnapshot>,
}

/// Encode `overlays`, stamped with the time they were exported.
pub(crate) fn encode(overlays: Vec<OverlaySnapshot>, now: u64) -> Result<Vec<u8>, String> {
    serde_ipld_dagcbor::to_vec(&Handoff {
        exported_at: now,
        overlays,
    })
    .map_err(|error| format!("encode overlay handoff: {error}"))
}

/// Decode a snapshot taken at most [`MAX_AGE_MS`] before `now`. `None` for a
/// stale, future-dated or unreadable one.
pub(crate) fn decode(bytes: &[u8], now: u64) -> Option<Vec<OverlaySnapshot>> {
    let handoff: Handoff = serde_ipld_dagcbor::from_slice(bytes).ok()?;
    let age = now.checked_sub(handoff.exported_at)?;
    (age <= MAX_AGE_MS).then_some(handoff.overlays)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod storage {
    use tonk_common::log;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Cache, CacheStorage, Response};

    use super::{decode, encode};
    use dialog_reactor::OverlaySnapshot;

    /// Outside the lifecycle cache grammar, so generation pruning keeps it.
    /// The JS shim's `activate` handler waits on the same name and key.
    const CACHE: &str = "TONK_OVERLAY_HANDOFF";
    const KEY: &str = "/__tonk/overlay-handoff";

    fn now() -> u64 {
        js_sys::Date::now() as u64
    }

    /// The global's `caches`, whether it is a service worker (production)
    /// or a window (this crate's browser tests).
    async fn cache() -> Result<Cache, JsValue> {
        let caches: CacheStorage =
            js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("caches"))?.dyn_into()?;
        JsFuture::from(caches.open(CACHE)).await?.dyn_into()
    }

    /// Store `overlays` for the successor, replacing any earlier snapshot.
    pub(crate) async fn save(overlays: Vec<OverlaySnapshot>) {
        let result: Result<(), JsValue> = async {
            let mut bytes = encode(overlays, now()).map_err(|error| JsValue::from_str(&error))?;
            let response = Response::new_with_opt_u8_array(Some(&mut bytes))?;
            JsFuture::from(cache().await?.put_with_str(KEY, &response)).await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            log!("overlay handoff: failed to save: {error:?}");
        }
    }

    /// Remove the stored snapshot and return it if it is still fresh.
    pub(crate) async fn take() -> Option<Vec<OverlaySnapshot>> {
        let result: Result<Option<Vec<u8>>, JsValue> = async {
            let cache = cache().await?;
            let found = JsFuture::from(cache.match_with_str(KEY)).await?;
            if found.is_undefined() {
                return Ok(None);
            }
            let response: Response = found.dyn_into()?;
            let buffer = JsFuture::from(response.array_buffer()?).await?;
            JsFuture::from(cache.delete_with_str(KEY)).await?;
            Ok(Some(js_sys::Uint8Array::new(&buffer).to_vec()))
        }
        .await;
        match result {
            Ok(bytes) => {
                let overlays = decode(&bytes?, now());
                if overlays.is_none() {
                    log!("overlay handoff: discarded a stale or unreadable snapshot");
                }
                overlays
            }
            Err(error) => {
                log!("overlay handoff: failed to read: {error:?}");
                None
            }
        }
    }
}
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) use storage::{save, take};

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use dialog_artifacts::{Changes, Update as _, Value};
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use dialog_reactor::BranchSession;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use crate::helpers::state::test_state;

    use super::*;

    fn overlays() -> Vec<OverlaySnapshot> {
        let mut changes = Changes::new();
        changes.associate(
            "test/status".parse().expect("attribute"),
            "tonk:test/handoff".parse().expect("entity"),
            Value::String("failed".into()),
        );
        vec![OverlaySnapshot {
            repository: None,
            branch: "main".into(),
            changes,
        }]
    }

    #[dialog_common::test]
    fn it_restores_a_fresh_snapshot() {
        let bytes = encode(overlays(), 1_000).expect("encode");
        assert_eq!(decode(&bytes, 1_000 + MAX_AGE_MS), Some(overlays()));
    }

    #[dialog_common::test]
    fn it_discards_a_stale_snapshot() {
        let bytes = encode(overlays(), 1_000).expect("encode");
        assert_eq!(decode(&bytes, 1_001 + MAX_AGE_MS), None);
    }

    #[dialog_common::test]
    fn it_discards_a_snapshot_from_the_future() {
        let bytes = encode(overlays(), 5_000).expect("encode");
        assert_eq!(decode(&bytes, 4_999), None);
    }

    #[dialog_common::test]
    fn it_discards_unreadable_bytes() {
        assert_eq!(decode(b"not a handoff", 0), None);
    }

    /// Overlays exported from one reactor, carried through the handoff
    /// encoding, land on the same branches of a fresh reactor over the same
    /// profile: what a successor worker's reactor starts as.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_restores_exported_overlays_into_a_fresh_reactor() {
        let tonk = test_state().await;
        let fact = overlays().remove(0).changes;
        let session: BranchSession = tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .expect("profile branch");
        session.state.assert_overlay(fact.clone());

        let exported = tonk.reactor.export_overlays();
        assert!(
            exported.iter().any(|snapshot| snapshot.repository.is_none()
                && snapshot.branch == tonk.active_branch
                && snapshot.changes.iter().eq(fact.iter())),
            "the profile branch's overlay is exported: {exported:?}"
        );

        let bytes = encode(exported.clone(), 0).expect("encode");
        let successor = crate::Reactor::new(tonk.profile.clone());
        let restored = successor
            .import_overlays(decode(&bytes, 0).expect("fresh"), &tonk.operator)
            .await;
        assert_eq!(restored, exported.len());
        assert_eq!(successor.export_overlays(), exported);
    }

    /// The successor takes the snapshot exactly once.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_hands_a_saved_snapshot_to_one_successor() {
        save(overlays()).await;
        assert_eq!(take().await, Some(overlays()));
        assert_eq!(take().await, None, "a taken snapshot is gone");
    }
}
