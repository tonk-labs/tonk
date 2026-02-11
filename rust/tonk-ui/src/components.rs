//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use leptos_router::location::{BrowserUrl, LocationProvider};
use wasm_bindgen::prelude::*;

use crate::api;

mod launcher;
use launcher::*;

mod toolbar;
use toolbar::*;

mod space;
use space::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = serviceWorkerActivates)]
    async fn service_worker_activates();

    /// Triggers a sync operation.
    /// Uses Background Sync API if available, otherwise falls back to /api/sync.
    #[wasm_bindgen(js_namespace = window, catch)]
    pub async fn sync() -> Result<(), JsValue>;
}

/// The current status of the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Service worker is still loading/activating, or setting up upstream.
    Loading,
    /// Service worker is ready and upstream remote is configured.
    Ready,
}

/// The root UI component for the Tonk application.
///
/// This component serves as the main entry point for the Tonk user interface,
/// rendering the primary application view.
///
/// On startup, it waits for the service worker to activate, then automatically
/// sets up the upstream remote if not already configured.
#[component]
pub fn TonkShell() -> impl IntoView {
    log!("Tonk shell initializing...");

    // Initialize the space: wait for SW, check status, setup remote if needed
    let init_resource = LocalResource::new(|| async {
        log!("Waiting for SW to activate...");
        service_worker_activates().await;
        log!("SW is activated, fetching status...");

        let status = api::status().await?;

        // If no upstream configured, add the remote (but don't set as upstream yet)
        if !status.has_upstream {
            log!("No upstream configured, adding remote...");
            api::authorize().await?;
            log!("Remote added successfully");
        }

        BrowserUrl::redirect(&format!("/space/{}", status.space_did));

        Ok::<_, crate::error::TonkUiError>(())
    });

    // Derive the application status from init resource
    let status = Signal::derive_local(move || {
        match init_resource.get() {
            Some(Ok(())) => Status::Ready,
            Some(Err(e)) => {
                log!("Initialization error: {:?}", e);
                // Still show as loading on error - could add an Error state later
                Status::Loading
            }
            None => Status::Loading,
        }
    });

    provide_context(status);

    view! {
        <TonkLauncher></TonkLauncher>
    }
}

#[cfg(test)]
mod tests {
    use crate::helpers::TestEnvironment;
    use anyhow::Result;
    #[cfg(not(target_arch = "wasm32"))]
    use thirtyfour::prelude::*;
    use tonk_worker::{StatusResponse, SyncResponse};

    #[dialog_common::test]
    async fn it_falls_back_to_index_for_unhandled_routes(env: TestEnvironment) -> Result<()> {
        // 1. Navigate to the root to confirm that the page loads
        let driver = env.driver().await?;
        let space = driver.query(By::Css(".space")).first().await?;
        assert!(space.text().await?.starts_with("did:key:"));

        // 2. Navigate again to /unhandled/route
        driver
            .goto(&format!("{}/unhandled/route", env.tonk_web))
            .await?;

        // 3. Confirm that the page loads (DID text rendered in the .space element)
        let space = driver.query(By::Css(".space")).first().await?;
        assert!(space.text().await?.starts_with("did:key:"));

        driver.quit().await?;
        Ok(())
    }

    /// Test that the UI auto-configures upstream on load via the access service.
    /// The access service is available at /ucan/ via Caddy reverse proxy.
    #[dialog_common::test]
    async fn it_configures_upstream(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for toolbar to become visible (indicates UI is ready and authorized)
        assert!(driver.query(By::Css(".toolbar.visible")).exists().await?);

        // Verify status shows upstream configured after auto-authorization
        let status_result = driver
            .execute(
                r#"
                const response = await fetch('/api/status');
                return await response.json();
                "#,
                vec![],
            )
            .await?;

        let status: StatusResponse = serde_json::from_value(status_result.json().clone())?;
        assert!(
            status.has_upstream,
            "Expected upstream to be configured after initialization"
        );

        driver.quit().await?;
        Ok(())
    }

    /// Test sync via /api/sync endpoint.
    #[dialog_common::test]
    async fn it_syncs_via_sync_route(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for toolbar to become visible (indicates UI is ready and authorized)
        assert!(driver.query(By::Css(".toolbar.visible")).exists().await?);

        // Perform sync
        let sync_result = driver
            .execute(
                r#"
                const response = await fetch('/api/sync', {
                    method: 'POST'
                });
                return await response.json();
                "#,
                vec![],
            )
            .await?;

        let sync_response: SyncResponse = serde_json::from_value(sync_result.json().clone())?;
        assert!(sync_response.success, "Sync should succeed");

        driver.quit().await?;
        Ok(())
    }

    /// Test sync via Background Sync API and verify data was pushed to remote.
    #[dialog_common::test]
    async fn it_syncs_via_background_sync_api(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for toolbar to become visible (indicates UI is ready and authorized)
        assert!(driver.query(By::Css(".toolbar.visible")).exists().await?);

        // Get the space_did from status (needed to verify sync on remote)
        let status_result = driver
            .execute(
                r#"
                const response = await fetch('/api/status');
                return await response.json();
                "#,
                vec![],
            )
            .await?;
        let status: StatusResponse = serde_json::from_value(status_result.json().clone())?;
        let space_did = status.space_did.clone();

        // Build the inspect URL (space_did needs URL-encoding since it contains ':')
        use url::form_urlencoded;
        let encoded_space_did: String =
            form_urlencoded::byte_serialize(space_did.as_bytes()).collect();

        let inspect_script = format!(
            r#"
            const response = await fetch('/api/inspect/site/origin/{}/branch/main');
            return await response.json();
            "#,
            encoded_space_did
        );

        // Verify remote branch has no revision before sync
        let inspect_result = driver.execute(&inspect_script, vec![]).await?;
        let branch_status: serde_json::Value =
            serde_json::from_value(inspect_result.json().clone())?;

        assert!(
            branch_status["revision"].is_null(),
            "Remote branch should have no revision before sync: {:?}",
            branch_status
        );

        // Register sync using the Background Sync API
        // This triggers the service worker's sync event handler
        driver
            .execute(
                r#"
                const registration = await navigator.serviceWorker.ready;
                await registration.sync.register('tonk-sync');
                "#,
                vec![],
            )
            .await?;

        // Verify data was pushed by checking the remote branch now has a revision
        let inspect_result = driver.execute(&inspect_script, vec![]).await?;
        let branch_status: serde_json::Value =
            serde_json::from_value(inspect_result.json().clone())?;

        assert!(
            branch_status["success"].as_bool().unwrap_or(false),
            "Remote branch resolution should succeed after sync: {:?}",
            branch_status
        );
        assert!(
            branch_status.get("revision").is_some() && !branch_status["revision"].is_null(),
            "Remote branch should have a revision after sync"
        );

        driver.quit().await?;
        Ok(())
    }
}
