//! Restore semantic account spot backups without inventing roster facts.
//! Best-effort: failures are isolated per spot and never block link or boot.

use std::sync::atomic::{AtomicBool, Ordering};

use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use tonk_account::backup::{AccountSpotSummary, SPACE_ROOT_SITE_PREFIX};
use tonk_common::log;

use crate::router::account_backup::{
    account_service_url, get_backed_up_spot, list_backed_up_spots,
};
use crate::worker::TonkState;

static RESTORE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Restore all backed-up spaces for the linked account.
pub(crate) async fn restore_spaces(tonk: &TonkState) {
    if RESTORE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(error) = try_restore_spaces(tonk).await {
        log!("restore skipped: {error}");
    }
    RESTORE_IN_FLIGHT.store(false, Ordering::SeqCst);
}

async fn try_restore_spaces(tonk: &TonkState) -> Result<(), crate::TonkWorkerError> {
    let Some(link) = crate::router::account::account_link(tonk).await else {
        return Ok(());
    };
    let Some(service) = account_service_url(tonk).await else {
        return Ok(());
    };
    let device = tonk.profile.signer().signer().clone();

    for spot in list_backed_up_spots(&device, &link, &service).await? {
        if spot.ambiguous {
            log!(
                "restore of subject '{}' skipped: backup is ambiguous",
                spot.subject
            );
            continue;
        }
        if let Err(error) = restore_one(tonk, &device, &link, &service, &spot).await {
            log!("restore of subject '{}' skipped: {error}", spot.subject);
        }
    }
    Ok(())
}

async fn restore_one(
    tonk: &TonkState,
    device: &dialog_credentials::Ed25519Signer,
    link: &DelegationChain,
    service: &str,
    spot: &AccountSpotSummary,
) -> Result<(), crate::TonkWorkerError> {
    let key = spot.key.as_deref().ok_or_else(|| {
        crate::TonkWorkerError::Internal("account spot has no selected backup key".to_string())
    })?;
    let expected: dialog_varsig::Did = spot.subject.parse().map_err(|error| {
        crate::TonkWorkerError::Internal(format!("account spot subject is invalid: {error:?}"))
    })?;

    if crate::router::join::find_replica_for_subject(tonk, &expected).await? {
        return Ok(());
    }

    let artifact = get_backed_up_spot(device, link, service, key).await?;
    let validated = artifact
        .validate_for(link.issuer())
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!("invalid backup artifact: {error}"))
        })?;
    if validated.subject != expected {
        return Err(crate::TonkWorkerError::Internal(
            "backup subject does not match its inventory row".to_string(),
        ));
    }
    let remote_url = artifact
        .remote_url
        .as_deref()
        .ok_or_else(|| crate::TonkWorkerError::Internal("backup has no sync remote".to_string()))?;

    let prefix_bytes = validated.chain.to_bytes().map_err(|error| {
        crate::TonkWorkerError::Internal(format!("serialize restored delegation: {error}"))
    })?;
    tonk.profile
        .access()
        .save(UcanDelegation(validated.chain))
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!("save restored delegation: {error}"))
        })?;
    tonk.profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{expected}"))
        .save(prefix_bytes)
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!("persist restored delegation: {error}"))
        })?;

    // The synced content branch remains authoritative for membership,
    // roles, names, invitations, and provenance.
    crate::router::join::mount_replica(
        tonk,
        &expected,
        Some(remote_url),
        artifact.revocation_url.as_deref(),
    )
    .await?;
    crate::router::repository::record_initialized_replica_in_profile(tonk, &expected).await?;
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal as _;
    use tonk_account::backup::AccountSpotBackup;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn artifact(
        root: &dialog_varsig::Did,
        seed: u8,
        name: Option<&str>,
    ) -> (dialog_varsig::Did, Vec<u8>) {
        let space = Ed25519Signer::import(&[seed; 32]).await.unwrap();
        let subject = space.did();
        let delegation = DelegationBuilder::new()
            .issuer(space)
            .audience(root)
            .subject(Subject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let backup = AccountSpotBackup {
            chain_hex: hex::encode(DelegationChain::new(delegation).to_bytes().unwrap()),
            remote_url: Some("https://sync.example.test/ucan/".to_string()),
            revocation_url: Some("https://relay.example.test/revocations/".to_string()),
            name: name.map(str::to_string),
        };
        (subject, serde_json::to_vec(&backup).unwrap())
    }

    struct InstalledInventory {
        _rows: crate::router::tests::GlobalPropertyGuard,
        _artifacts: crate::router::tests::GlobalPropertyGuard,
        _fetch: crate::router::tests::GlobalPropertyGuard,
    }

    fn install_inventory(rows: &[AccountSpotSummary], artifacts: &[Vec<u8>]) -> InstalledInventory {
        let rows = JsValue::from_str(&serde_json::to_string(rows).unwrap());
        let rows = crate::router::tests::GlobalPropertyGuard::replace("__tonkRestoreRows", &rows);
        let values = js_sys::Array::new();
        for artifact in artifacts {
            values.push(&js_sys::Uint8Array::from(artifact.as_slice()));
        }
        let artifacts = crate::router::tests::GlobalPropertyGuard::replace(
            "__tonkRestoreArtifacts",
            values.as_ref(),
        );
        let fetch = js_sys::Function::new_with_args(
            "request",
            r#"
            if (request.url.endsWith('/chains/list')) {
                return Promise.resolve(new Response('[]', {
                    status: 200,
                    headers: { 'X-Tonk-Account-Spots': 'v1' }
                }));
            }
            if (request.url.endsWith('/chains/spots')) {
                return Promise.resolve(new Response(
                    new TextEncoder().encode(globalThis.__tonkRestoreRows),
                    { status: 200 }
                ));
            }
            if (request.url.endsWith('/chains/get')) {
                const artifact = globalThis.__tonkRestoreArtifacts.shift();
                return Promise.resolve(new Response(artifact, { status: 200 }));
            }
            return Promise.resolve(new Response('{}', { status: 500 }));
            "#,
        );
        let fetch = crate::router::tests::GlobalPropertyGuard::replace("fetch", fetch.as_ref());
        InstalledInventory {
            _rows: rows,
            _artifacts: artifacts,
            _fetch: fetch,
        }
    }

    #[dialog_common::test]
    async fn it_restores_legacy_unnamed_and_isolates_ambiguous_and_bad_artifacts() {
        let state = crate::router::tests::test_state().await;
        let link = crate::router::account::account_link(&state).await.unwrap();
        let root = link.issuer().clone();
        let (legacy_subject, legacy) = artifact(&root, 101, None).await;
        let (good_subject, good) = artifact(&root, 102, Some("garden")).await;
        let bad_subject = Ed25519Signer::import(&[103; 32]).await.unwrap().did();
        let ambiguous_subject = Ed25519Signer::import(&[104; 32]).await.unwrap().did();
        let rows = vec![
            AccountSpotSummary {
                subject: ambiguous_subject.to_string(),
                key: None,
                name: None,
                remote_url: None,
                revocation_url: None,
                ambiguous: true,
            },
            AccountSpotSummary {
                subject: bad_subject.to_string(),
                key: Some("bad".to_string()),
                name: Some("bad".to_string()),
                remote_url: Some("https://sync.example.test/ucan/".to_string()),
                revocation_url: None,
                ambiguous: false,
            },
            AccountSpotSummary {
                subject: legacy_subject.to_string(),
                key: Some("legacy".to_string()),
                name: None,
                remote_url: Some("https://sync.example.test/ucan/".to_string()),
                revocation_url: None,
                ambiguous: false,
            },
            AccountSpotSummary {
                subject: good_subject.to_string(),
                key: Some("good".to_string()),
                name: Some("garden".to_string()),
                remote_url: Some("https://sync.example.test/ucan/".to_string()),
                revocation_url: None,
                ambiguous: false,
            },
        ];
        let _inventory = install_inventory(&rows, &[b"bad artifact".to_vec(), legacy, good]);

        restore_spaces(&state).await;

        assert!(
            crate::router::join::find_replica_for_subject(&state, &legacy_subject)
                .await
                .unwrap()
        );
        assert!(
            crate::router::join::find_replica_for_subject(&state, &good_subject)
                .await
                .unwrap()
        );
        assert!(
            !crate::router::join::find_replica_for_subject(&state, &bad_subject)
                .await
                .unwrap()
        );
        assert!(
            !crate::router::join::find_replica_for_subject(&state, &ambiguous_subject)
                .await
                .unwrap()
        );
    }
}
