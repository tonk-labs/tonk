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

/// The subject DID of the currently viewed space. `None` when no
/// space is loaded (or still loading). Updated by [`TonkSpace`]
/// when its [`RepositoryInfo`] resolves; consumed by the sidebar
/// toolbar to render a matching sigil.
pub type ActiveSubject = RwSignal<Option<String>, LocalStorage>;

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

    // Initialize the space: wait for SW, ensure the default repo
    // exists, then (if the user landed on `/`) redirect into it.
    // The worker no longer auto-creates anything at startup — we
    // `PUT` here with `If-None-Match: *`, which covers both
    // "didn't exist, created" (201) and "already existed" (412) as
    // success. Redirect only fires when the current path is `/`
    // so deep links like `/space/home/branch/main` are respected.
    let init_resource = LocalResource::new(|| async {
        log!("Waiting for SW to activate...");
        service_worker_activates().await;
        log!("SW is activated, ensuring default repository...");

        api::init().await?;

        let pathname = window()
            .location()
            .pathname()
            .unwrap_or_else(|_| "/".to_string());

        if pathname == "/" {
            BrowserUrl::redirect(&format!("/space/{}", api::DEFAULT_REPO));
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

    let active_subject: ActiveSubject = RwSignal::new_local(None);
    provide_context(active_subject);

    view! {
        <TonkLauncher></TonkLauncher>
    }
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use crate::helpers::TestEnvironment;
    use anyhow::Result;
    #[cfg(not(target_arch = "wasm32"))]
    use thirtyfour::prelude::*;
    use tonk_worker::{RepositoryInfo, SyncResponse};

    #[dialog_common::test]
    async fn it_falls_back_to_index_for_unhandled_routes(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // 1. Landing on `/` redirects into `/space/home` and renders
        //    the repository JSON once the worker is up.
        let repository = driver.query(By::Css("pre.repository")).first().await?;
        assert!(
            repository.text().await?.contains("did:key:"),
            "expected repository JSON to include a did:key value",
        );

        // 2. Navigate to an unmatched route. The SPA router's
        //    fallback should render the 404 section instead of
        //    redirecting — deep links to unknown paths must not
        //    silently rewrite to `/space/home`.
        driver
            .goto(&format!("{}/unhandled/route", env.tonk_web))
            .await?;

        // The fallback's class is `404`, which isn't a legal CSS
        // identifier on its own — use an attribute selector
        // instead.
        let fallback = driver
            .query(By::Css(r#"section[class="404"]"#))
            .first()
            .await?;
        assert!(
            fallback.text().await?.contains("Nothing here"),
            "expected 404 fallback to render for unknown paths",
        );

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

        // Verify the default branch has an upstream after auto-authorization.
        let info_result = driver
            .execute(
                r#"
                const response = await fetch('/api/repository/home');
                return await response.json();
                "#,
                vec![],
            )
            .await?;

        let info: RepositoryInfo = serde_json::from_value(info_result.json().clone())?;
        let main = info.branch.get("main").expect("main branch present");
        assert!(
            main.upstream.is_some(),
            "Expected main branch to have an upstream after initialization"
        );

        driver.quit().await?;
        Ok(())
    }

    /// Test sync via the sync endpoint.
    #[dialog_common::test]
    async fn it_syncs_via_sync_route(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Wait for toolbar to become visible (indicates UI is ready and authorized)
        assert!(driver.query(By::Css(".toolbar.visible")).exists().await?);

        // Perform sync
        let sync_result = driver
            .execute(
                r#"
                const response = await fetch('/api/repository/home/branch/main/sync', {
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

        let inspect_script = r#"
            const response = await fetch('/api/inspect/repository/home/remote/origin/branch/main');
            return await response.json();
        "#;

        // Check remote branch state before sync
        let _inspect_result = driver.execute(inspect_script, vec![]).await?;

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
        let inspect_result = driver.execute(inspect_script, vec![]).await?;
        let branch_status: serde_json::Value =
            serde_json::from_value(inspect_result.json().clone())?;

        assert!(
            branch_status["success"].as_bool().unwrap_or(false),
            "Remote branch resolution should succeed after sync: {:?}",
            branch_status
        );

        driver.quit().await?;
        Ok(())
    }
}
