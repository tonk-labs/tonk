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

/// The snapshot as stored: which build wrote it, when, and what it holds.
#[derive(Debug, Serialize, Deserialize)]
struct Handoff {
    build: String,
    exported_at: u64,
    overlays: Vec<OverlaySnapshot>,
}

/// What a booting worker does with a stored snapshot.
#[derive(Debug, PartialEq)]
pub(crate) enum Received {
    /// A fresh snapshot from a predecessor: restore it.
    Restore(Vec<OverlaySnapshot>),
    /// This build wrote it while retiring and was then restarted before its
    /// successor took over. It belongs to the successor, so leave it.
    Own,
    /// Stale, future-dated or unreadable: drop it.
    Discard,
}

/// Encode `overlays`, stamped with the exporting `build` and the time.
pub(crate) fn encode(
    overlays: Vec<OverlaySnapshot>,
    build: &str,
    now: u64,
) -> Result<Vec<u8>, String> {
    serde_ipld_dagcbor::to_vec(&Handoff {
        build: build.to_owned(),
        exported_at: now,
        overlays,
    })
    .map_err(|error| format!("encode overlay handoff: {error}"))
}

/// Decide what the worker running `build` does with `bytes` at `now`.
pub(crate) fn decode(bytes: &[u8], build: &str, now: u64) -> Received {
    let Ok(handoff) = serde_ipld_dagcbor::from_slice::<Handoff>(bytes) else {
        return Received::Discard;
    };
    if handoff.build == build {
        return Received::Own;
    }
    match now.checked_sub(handoff.exported_at) {
        Some(age) if age <= MAX_AGE_MS => Received::Restore(handoff.overlays),
        _ => Received::Discard,
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod storage {
    use tonk_common::log;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Cache, CacheStorage, Response};

    use super::{Received, decode, encode};
    use dialog_reactor::OverlaySnapshot;

    /// Outside the lifecycle cache grammar, so generation pruning keeps it.
    /// The JS shim's `activate` handler waits on the same name and key.
    const CACHE: &str = "TONK_OVERLAY_HANDOFF";
    const KEY: &str = "/__tonk/overlay-handoff";

    fn now() -> u64 {
        js_sys::Date::now() as u64
    }

    fn build() -> String {
        crate::cache::current_build_id().unwrap_or_else(|| "dev".to_owned())
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
            let mut bytes =
                encode(overlays, &build(), now()).map_err(|error| JsValue::from_str(&error))?;
            let response = Response::new_with_opt_u8_array(Some(&mut bytes))?;
            JsFuture::from(cache().await?.put_with_str(KEY, &response)).await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            log!("overlay handoff: failed to save: {error:?}");
        }
    }

    /// Take a predecessor's snapshot, removing it, if it is still fresh. A
    /// snapshot this build wrote itself stays for the successor.
    pub(crate) async fn take() -> Option<Vec<OverlaySnapshot>> {
        take_as(&build()).await
    }

    /// [`take`] on behalf of the worker running `build`.
    pub(super) async fn take_as(build: &str) -> Option<Vec<OverlaySnapshot>> {
        let result: Result<Option<Vec<OverlaySnapshot>>, JsValue> = async {
            let cache = cache().await?;
            let found = JsFuture::from(cache.match_with_str(KEY)).await?;
            if found.is_undefined() {
                return Ok(None);
            }
            let response: Response = found.dyn_into()?;
            let buffer = JsFuture::from(response.array_buffer()?).await?;
            let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            let overlays = match decode(&bytes, build, now()) {
                Received::Own => return Ok(None),
                Received::Restore(overlays) => Some(overlays),
                Received::Discard => {
                    log!("overlay handoff: discarded a stale or unreadable snapshot");
                    None
                }
            };
            JsFuture::from(cache.delete_with_str(KEY)).await?;
            Ok(overlays)
        }
        .await;
        result.unwrap_or_else(|error| {
            log!("overlay handoff: failed to read: {error:?}");
            None
        })
    }
}
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
use storage::take_as;
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
        let bytes = encode(overlays(), "a", 1_000).expect("encode");
        assert_eq!(
            decode(&bytes, "b", 1_000 + MAX_AGE_MS),
            Received::Restore(overlays())
        );
    }

    #[dialog_common::test]
    fn it_leaves_its_own_snapshot_for_the_successor() {
        let bytes = encode(overlays(), "a", 1_000).expect("encode");
        assert_eq!(decode(&bytes, "a", 1_000), Received::Own);
    }

    #[dialog_common::test]
    fn it_discards_a_stale_snapshot() {
        let bytes = encode(overlays(), "a", 1_000).expect("encode");
        assert_eq!(decode(&bytes, "b", 1_001 + MAX_AGE_MS), Received::Discard);
    }

    #[dialog_common::test]
    fn it_discards_a_snapshot_from_the_future() {
        let bytes = encode(overlays(), "a", 5_000).expect("encode");
        assert_eq!(decode(&bytes, "b", 4_999), Received::Discard);
    }

    #[dialog_common::test]
    fn it_discards_unreadable_bytes() {
        assert_eq!(decode(b"not a handoff", "b", 0), Received::Discard);
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
        // The profile's overlay also carries what the worker publishes at
        // boot (the local-root rows), so the fact is one entry among others.
        assert!(
            exported.iter().any(|snapshot| snapshot.repository.is_none()
                && snapshot.branch == tonk.active_branch
                && fact
                    .iter()
                    .all(|entry| snapshot.changes.iter().any(|exported| exported == entry))),
            "the profile branch's overlay is exported: {exported:?}"
        );

        let bytes = encode(exported.clone(), "predecessor", 0).expect("encode");
        let successor = crate::Reactor::new(tonk.profile.clone());
        let restored = successor
            .import_overlays(
                match decode(&bytes, "successor", 0) {
                    Received::Restore(overlays) => overlays,
                    other => panic!("expected a snapshot to restore, got {other:?}"),
                },
                &tonk.operator,
            )
            .await;
        assert_eq!(restored, exported.len());
        assert_eq!(successor.export_overlays(), exported);
    }

    /// The successor takes the snapshot exactly once, and the incumbent
    /// that wrote it, restarted before handing over, leaves it alone.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_hands_a_saved_snapshot_to_one_successor() {
        save(overlays()).await;
        assert_eq!(take().await, None, "the writer does not take its own");
        assert_eq!(take_as("successor").await, Some(overlays()));
        assert_eq!(take_as("successor").await, None, "a taken snapshot is gone");
    }
}
