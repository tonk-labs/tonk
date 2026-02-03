//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
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
    use tonk_worker::{
        AuthorizeRequest, AuthorizeResponse, RemoteBranchStatusResponse, StatusResponse,
        SyncResponse,
    };

    /// Test that the UI loads and the service worker responds to status requests.
    #[dialog_common::test]
    async fn it_loads_ui_and_gets_status(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for service worker to activate before making API calls
        driver
            .execute("await window.serviceWorkerActivates();", vec![])
            .await?;

        // Execute JavaScript to call the status API
        let result = driver
            .execute(
                r#"
                const response = await fetch('/api/status');
                return await response.json();
                "#,
                vec![],
            )
            .await?;

        let status: StatusResponse = serde_json::from_value(result.json().clone())?;

        // Status should show no upstream configured initially
        assert!(!status.has_upstream, "Expected no upstream initially");
        assert!(!status.space_did.is_empty(), "Expected space_did to be set");
        assert!(
            !status.operator_did.is_empty(),
            "Expected operator_did to be set"
        );

        driver.quit().await?;
        Ok(())
    }

    /// Test authorization flow through the browser with access service.
    #[dialog_common::test]
    async fn it_authorizes_via_browser(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for service worker to activate before making API calls
        driver
            .execute("await window.serviceWorkerActivates();", vec![])
            .await?;

        // Build authorize request with access service URL
        let authorize_request = AuthorizeRequest {
            access_service_url: Some(env.access_service_url()),
        };
        let request_json = serde_json::to_string(&authorize_request)?;

        // Execute authorization via browser
        let script = format!(
            r#"
            const response = await fetch('/api/authorize', {{
                method: 'POST',
                headers: {{ 'Content-Type': 'application/json' }},
                body: '{}'
            }});
            return await response.json();
            "#,
            request_json.replace('\'', "\\'")
        );

        let result = driver.execute(&script, vec![]).await?;
        let auth_response: AuthorizeResponse = serde_json::from_value(result.json().clone())?;

        assert!(auth_response.success, "Authorization should succeed");
        assert!(
            auth_response.error.is_none(),
            "Authorization should not have error"
        );

        // Verify status now shows upstream configured
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
            "Expected upstream to be configured after authorization"
        );

        driver.quit().await?;
        Ok(())
    }

    /// Test full sync flow through the browser.
    #[dialog_common::test]
    async fn it_syncs_via_browser(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for service worker to activate before making API calls
        driver
            .execute("await window.serviceWorkerActivates();", vec![])
            .await?;

        // First authorize with the access service
        let authorize_request = AuthorizeRequest {
            access_service_url: Some(env.access_service_url()),
        };
        let request_json = serde_json::to_string(&authorize_request)?;

        let auth_script = format!(
            r#"
            const response = await fetch('/api/authorize', {{
                method: 'POST',
                headers: {{ 'Content-Type': 'application/json' }},
                body: '{}'
            }});
            return await response.json();
            "#,
            request_json.replace('\'', "\\'")
        );

        let auth_result = driver.execute(&auth_script, vec![]).await?;
        let auth_response: AuthorizeResponse = serde_json::from_value(auth_result.json().clone())?;
        assert!(auth_response.success, "Authorization should succeed");

        // Now perform sync
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
}
