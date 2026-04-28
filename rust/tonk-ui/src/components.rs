//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use leptos_router::location::{BrowserUrl, LocationProvider};
use tonk_worker::{Notification, ProfileInfo};
use wasm_bindgen::prelude::*;

use crate::{api, error::TonkUiError, watch::watch};

mod launcher;
use launcher::*;

mod toolbar;
use toolbar::*;

mod space;
use space::*;

mod profile;
use profile::*;

mod create_space;
use create_space::*;

mod join;
use join::*;

mod invite;
use invite::*;

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

/// Shared [`LocalResource`] holding the latest `GET /api/profile`
/// response. Provided by [`TonkShell`] so every consumer (today:
/// the sidebar toolbar and profile view) reads from one source of
/// truth. The shell refetches the resource automatically whenever
/// the worker broadcasts on `/api/profile`, so writes from any
/// source (this tab, another tab, external fetch) flow through.
///
/// `Ok(None)` is used to model "not yet ready" (shell still
/// initialising); `Ok(Some(info))` is a successful fetch.
pub type ProfileResource = LocalResource<Result<Option<ProfileInfo>, TonkUiError>>;

/// Shared open-state for the create-space dialog. Flipped to
/// `true` by the sidebar's `+` tile; flipped back to `false` by
/// the dialog itself on cancel, successful create, or when the
/// user dismisses via Esc / click-outside.
pub type CreateSpaceOpen = RwSignal<bool>;

/// Shared open-state for the invite dialog. `Some(name)` opens
/// the dialog and triggers a fresh invite mint for that space;
/// `None` closes it. Set by the `Invite` button in [`TonkSpace`].
pub type InviteSpace = RwSignal<Option<String>>;

/// Outcome of the most recent invite redemption, if any. Written
/// by [`TonkJoin`] when a join completes; rendered as a
/// `data-last-join-outcome` attribute on the launcher's root
/// element so tests and any future banner / toast UI can react
/// to which path ran without parsing an HTTP response.
///
/// `None` when no join has happened yet this session.
pub type LastJoinOutcome = RwSignal<Option<&'static str>>;

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

    // Fire the profile fetch as soon as the shell reports
    // `Status::Ready`. Gating on `Ready` avoids the same
    // deep-link / service-worker race that affects `TonkSpace`.
    // Sharing the resource via context means a single fetch
    // feeds the sidebar and the create-space flow — the latter
    // calls `.refetch()` after a successful PUT so the sidebar
    // picks up the new tile without us plumbing a second signal.
    let profile_resource: ProfileResource = LocalResource::new(move || {
        let ready = status.get() == Status::Ready;
        async move {
            if !ready {
                return Ok(None);
            }
            api::profile().await.map(Some)
        }
    });
    provide_context(profile_resource);

    // The worker posts on `/api/profile` whenever the profile
    // repo's meta branch commits (replica added/removed, remote
    // edited, etc.). Refetching on any message keeps the sidebar
    // in sync with writes from anywhere — this tab, another tab,
    // or a direct fetch that bypasses the dialog. The payload
    // carries the new revision; we ignore it today (refetch is
    // cheap and the endpoint is the source of truth) but the
    // shape is available for future dedup.
    let profile_update = watch::<Notification>("/api/profile");
    Effect::new(move |_| {
        if profile_update.get().is_some() {
            profile_resource.refetch();
        }
    });

    // Shared open-state for the create-space dialog. The `+`
    // tile in the sidebar flips this to `true`; the dialog
    // itself resets it on close.
    let create_space_open: CreateSpaceOpen = RwSignal::new(false);
    provide_context(create_space_open);

    // Shared open-state for the invite dialog. The space view's
    // "Invite" button writes a `Some(name)` here; the dialog
    // resets it back to `None` on close.
    let invite_space: InviteSpace = RwSignal::new(None);
    provide_context(invite_space);

    // The last join outcome lives in a shared signal so the view
    // tree (specifically `TonkLauncher`) can render it as a
    // `data-last-join-outcome` attribute through the regular
    // reactive path — no manual DOM mutation from inside a
    // component.
    let last_join_outcome: LastJoinOutcome = RwSignal::new(None);
    provide_context(last_join_outcome);

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

        // 1. Landing on `/` redirects into `/space/home`. The
        //    banner title renders once `repository_view` resolves,
        //    so it doubles as a "shell is ready" signal. Its
        //    `title` attribute carries the subject DID.
        let title = driver.query(By::Css(".space-banner-title")).first().await?;
        assert_eq!(title.text().await?, "home");
        let subject = title.attr("title").await?.unwrap_or_default();
        assert!(
            subject.starts_with("did:key:"),
            "expected banner title attribute to be a did:key, got: {subject}",
        );

        // 2. Navigate to an unmatched route. The SPA router's
        //    fallback should render the 404 section instead of
        //    redirecting — deep links to unknown paths must not
        //    silently rewrite to `/space/home`.
        driver
            .goto(&format!("{}/unhandled/route", env.tonk_web))
            .await?;

        let fallback = driver.query(By::Css("section.not-found")).first().await?;
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

        // Wait for the home space banner title to render. This only
        // appears after the service worker is active, `PUT
        // /api/repository/home` completed, and the repository
        // fetch resolved — a strict superset of the old
        // `.toolbar.visible` readiness signal.
        driver.query(By::Css(".space-banner-title")).first().await?;

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

        // Wait for the home space banner title to render. This only
        // appears after the service worker is active, `PUT
        // /api/repository/home` completed, and the repository
        // fetch resolved — a strict superset of the old
        // `.toolbar.visible` readiness signal.
        driver.query(By::Css(".space-banner-title")).first().await?;

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

        // Wait for the home space banner title to render. This only
        // appears after the service worker is active, `PUT
        // /api/repository/home` completed, and the repository
        // fetch resolved — a strict superset of the old
        // `.toolbar.visible` readiness signal.
        driver.query(By::Css(".space-banner-title")).first().await?;

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

    /// Test the `<tonk-code>` editor mounts inside the default
    /// branch row and exercises its public contract end-to-end:
    /// `value` round-trip, `change` event firing on user input,
    /// and inline lint markers from a real LSP round-trip.
    ///
    /// Drives the actual built bundle (via Trunk-served
    /// `assets/tonk-code/*`) through chromedriver, so this test
    /// catches both the Rust-side install path and any drift in
    /// the JS contract.
    #[dialog_common::test]
    async fn it_renders_tonk_code_editor(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // The default chromedriver script-timeout is 30s; our
        // polling loops cap at 10s each but bump the budget so
        // the cumulative chain doesn't trip an outer timeout.
        driver
            .set_script_timeout(std::time::Duration::from_secs(30))
            .await?;

        // Wait for the home space to render. The `main` branch
        // row is force-expanded for solo branches (and `main` is
        // the default-open name regardless), so the editor
        // mounts on its own without us toggling anything.
        driver.query(By::Css(".space-banner-title")).first().await?;

        // Wait for the editor to mount. CodeMirror lives inside
        // the element's shadow root (open mode), so we poll
        // `el.shadowRoot.querySelector(".cm-content")` instead
        // of a light-DOM selector. Also poll for `el` itself
        // first — Leptos may not have inserted the element when
        // the banner-title query above resolved.
        driver
            .execute(
                r#"
                const start = performance.now();
                while (performance.now() - start < 10000) {
                    const el = document.querySelector("tonk-code");
                    if (el && el.shadowRoot && el.shadowRoot.querySelector(".cm-content")) {
                        return null;
                    }
                    await new Promise((resolve) => setTimeout(resolve, 50));
                }
                throw new Error("tonk-code .cm-content never appeared");
                "#,
                vec![],
            )
            .await?;

        // (a) Value round-trip — set via property, read back.
        let value_back = driver
            .execute(
                r#"
                const el = document.querySelector("tonk-code");
                el.value = "person-name:\n  attribute:\n    the: io.gozala.person/name\n";
                return el.value;
                "#,
                vec![],
            )
            .await?;
        assert_eq!(
            value_back.json().as_str().unwrap_or(""),
            "person-name:\n  attribute:\n    the: io.gozala.person/name\n",
            "value getter should round-trip the setter",
        );

        // (b) Change event — programmatic `.value` writes don't
        // refire `change`, so we dispatch into CodeMirror's
        // contentEditable surface inside the shadow root via
        // `execCommand("insertText")`, which the editor handles
        // exactly like a keystroke.
        let captured = driver
            .execute(
                r#"
                const el = document.querySelector("tonk-code");
                const target = el.shadowRoot.querySelector(".cm-content");
                target.focus();
                let captured = null;
                const listener = (ev) => { captured = ev.detail.value; };
                el.addEventListener("change", listener);
                document.execCommand("insertText", false, "X");
                // Give CodeMirror a frame to flush the change event.
                await new Promise((resolve) => setTimeout(resolve, 50));
                el.removeEventListener("change", listener);
                return captured;
                "#,
                vec![],
            )
            .await?;
        assert!(
            captured.json().is_string(),
            "change event should fire with a string value (got {:?})",
            captured.json()
        );

        // (c) Diagnostics — write deliberately invalid YAML and
        // wait for the LSP round-trip to materialize a
        // CodeMirror lint marker. This exercises the full
        // pipeline: editor → LSP transport →
        // tonk-language-server → tonk-notation diagnostics →
        // push back → CodeMirror lint.
        //
        // The editor's LSP connection is lazy — `language-server`
        // attribute arms it but the SSE channel doesn't open
        // until first focus. The change-event step above already
        // focused the editor; if it hadn't, this would silently
        // never produce diagnostics.
        let diag_count = driver
            .execute(
                r#"
                const el = document.querySelector("tonk-code");
                el.value = "not-valid-yaml: : :\n  bad: stuff: here\n";
                // Lint markers live inside the shadow root.
                const start = performance.now();
                while (performance.now() - start < 10000) {
                    const markers = el.shadowRoot.querySelectorAll(
                        ".cm-lintRange-error, .cm-lintRange-warning"
                    );
                    if (markers.length > 0) return markers.length;
                    await new Promise((resolve) => setTimeout(resolve, 100));
                }
                return 0;
                "#,
                vec![],
            )
            .await?;
        assert!(
            diag_count.json().as_u64().unwrap_or(0) > 0,
            "expected at least one inline lint marker after a bad YAML edit (got {:?})",
            diag_count.json()
        );

        driver.quit().await?;
        Ok(())
    }
}
