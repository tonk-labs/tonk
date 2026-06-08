//! User interface components for the Tonk application.
//!
//! This crate provides the web-based UI for Tonk, built using the Leptos framework.
//! It compiles to Wasm and runs in the browser.

use leptos::{logging::log, prelude::*};
use tonk_worker::{Notification, ProfileInfo};

use crate::{api, error::TonkUiError, watch::watch};

pub(crate) mod route;

mod launcher;
use launcher::*;

mod toolbar;
use toolbar::*;

mod space;
use space::*;

mod concept;
use concept::*;

mod display;
use display::*;

mod hub;
use hub::*;

mod layout;
use layout::*;

mod board;
use board::*;

mod inspector;
pub use inspector::register as register_inspector;

mod profile;
use profile::*;

mod join;
use join::*;

mod invite;
use invite::*;

/// The hosting document's service-worker Client ID, learned from
/// the `X-Tonk-Client-Id` header on the `PUT /api/repository/...`
/// response. Provided as a Leptos context so descendant
/// components can embed it in iframe URLs for the host/guest
/// bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostId(pub String);

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

/// Shared open-state for the invite dialog. `Some(name)` opens
/// the dialog and triggers a fresh invite mint for that space;
/// `None` closes it. Set by the workspace's `<tonk-share>` control
/// (via the `tonk:share` window bridge in [`TonkShell`]) and by the
/// space viewer's share button.
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

    // Initialize the space: ensure the default repo exists, then
    // (if the user landed on `/`) redirect into it. The worker
    // doesn't auto-create anything at startup — we `PUT` here
    // with `If-None-Match: *`, which covers both "didn't exist,
    // created" (201) and "already existed" (412) as success.
    // Redirect only fires when the current path is `/` so deep
    // links like `/space/home/branch/main` are respected.
    //
    // SW readiness is gated inside the API layer
    // (`tonk_host::ready::wait`) so this resource doesn't need
    // its own `serviceWorkerActivates()` step.
    let init_resource = LocalResource::new(|| async {
        log!("Ensuring default repository...");

        // `/` renders the Tonk Hub (a picker over the profile's spaces),
        // so there is no longer a redirect into the default space — the
        // user lands on the Hub and chooses where to go.
        let host_id = api::init().await?;

        Ok::<_, crate::error::TonkUiError>(host_id)
    });

    // Publish the host id as a reactive context so descendant
    // components can subscribe: `None` while init is in flight
    // or has errored, `Some(HostId)` once the PUT succeeds.
    let host_id =
        Signal::derive_local(move || init_resource.get().and_then(|r| r.ok()).map(HostId));

    provide_context(host_id);

    // Fire the profile fetch eagerly. The API layer's SW gate
    // holds the request until the worker is up, so we don't
    // need a separate ready signal at the shell level. Sharing
    // the resource via context means a single fetch feeds the
    // sidebar and the create-space flow — the latter calls
    // `.refetch()` after a successful PUT so the sidebar picks
    // up the new tile without us plumbing a second signal.
    let profile_resource: ProfileResource =
        LocalResource::new(|| async { api::profile().await.map(Some) });
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

    // Shared open-state for the invite dialog. The workspace top
    // bar's `<tonk-share>` control writes a `Some(name)` here (via
    // the `tonk:share` bridge below); the dialog resets it back to
    // `None` on close.
    let invite_space: InviteSpace = RwSignal::new(None);
    provide_context(invite_space);

    // Bridge `<tonk-share>` to the invite dialog. The workspace
    // element can't call into the shell directly — view templates
    // bind DOM events to data-model commands only, and sharing isn't
    // a data mutation (it mints a UCAN invite over HTTP and opens a
    // modal). So the element dispatches a bubbling, composed
    // `tonk:share` CustomEvent carrying `{ repo }`, and the shell
    // listens on the window. This mirrors how the sync controller
    // bridges `tonk:committed` / `tonk:status-refresh`.
    let _ = leptos_use::use_event_listener(
        window(),
        leptos::ev::Custom::<web_sys::CustomEvent>::new("tonk:share"),
        move |event| {
            // Prefer the repo the element resolved from its
            // `<tonk-repository>` ancestor; fall back to the active
            // repo parsed from the route for any host that fires the
            // event without a detail.
            let repo = share_repo_from_event(&event).or_else(active_repo_from_route);
            if let Some(repo) = repo {
                invite_space.set(Some(repo));
            }
        },
    );

    // The last join outcome lives in a shared signal so the view
    // tree (specifically `TonkLauncher`) can render it as a
    // `data-last-join-outcome` attribute through the regular
    // reactive path — no manual DOM mutation from inside a
    // component.
    let last_join_outcome: LastJoinOutcome = RwSignal::new(None);
    provide_context(last_join_outcome);

    view! {
        // App-wide LSP transport. One <tonk-diagnostics-provider>
        // wraps the entire shell so every <tonk-code> editor in
        // the app — across spaces, branches, dialogs — shares
        // one LSP client. Editors announce themselves via
        // `tonk-code-connect`; the provider attaches its client
        // to each. Spaces correspond to repositories, so this
        // also gives the language server a single transport
        // through which it sees every repository's documents.
        <tonk-diagnostics-provider>
            <TonkLauncher></TonkLauncher>
        </tonk-diagnostics-provider>
    }
}

/// Read the `repo` from a `tonk:share` event's `detail`, if present
/// and non-empty.
fn share_repo_from_event(event: &web_sys::CustomEvent) -> Option<String> {
    js_sys::Reflect::get(&event.detail(), &wasm_bindgen::JsValue::from_str("repo"))
        .ok()
        .and_then(|value| value.as_string())
        .filter(|repo| !repo.is_empty())
}

/// Fall back to the active repository parsed from the current route
/// (`/space/{branch}@{name}/…`) when a `tonk:share` event carries no
/// repo of its own. Mirrors the toolbar's active-space derivation.
fn active_repo_from_route() -> Option<String> {
    let pathname = window().location().pathname().ok()?;
    let segment = pathname.strip_prefix("/space/")?.split('/').next()?;
    route::parse_space(segment).map(|space| space.name)
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

        // 1. Landing on `/` redirects into `/space/home`. The bare
        //    display route mounts `tonk-repository.display-route`
        //    once the space is ready, so it doubles as a "shell is
        //    ready" signal.
        let repository = driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;
        assert_eq!(
            driver.current_url().await?.path(),
            "/space/home",
            "expected `/` to redirect to /space/home",
        );
        assert_eq!(
            repository.attr("name").await?.unwrap_or_default(),
            "home",
            "expected the display route to name the home repository",
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

        // Wait for the home space's bare display route to mount.
        // `tonk-repository.display-route` only appears after the
        // service worker is active, `PUT /api/repository/home`
        // completed, and the redirect to /space/home landed.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

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

        // Wait for the home space's bare display route to mount.
        // `tonk-repository.display-route` only appears after the
        // service worker is active, `PUT /api/repository/home`
        // completed, and the redirect to /space/home landed.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

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

        // Wait for the home space's bare display route to mount.
        // `tonk-repository.display-route` only appears after the
        // service worker is active, `PUT /api/repository/home`
        // completed, and the redirect to /space/home landed.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

        let inspect_script = r#"
            const response = await fetch('/api/inspect/repository/home/remote/origin/branch/main');
            return await response.json();
        "#;

        // Check remote branch state before sync
        let _inspect_result = driver.execute(inspect_script, vec![]).await?;

        // Register sync using the Background Sync API. The tag carries
        // the repository identity (`tonk-sync:{repo}`) so the worker's
        // `sync` event handler knows which repo's upstream branches to
        // sweep.
        driver
            .execute(
                r#"
                const registration = await navigator.serviceWorker.ready;
                await registration.sync.register('tonk-sync:home');
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

    /// A local write reaches the `origin` remote on its own — no
    /// Pull/Push button — once the background sync controller's
    /// commit trigger fires. Exercises the default-on controller end
    /// to end: commit locally, signal it the way a real editor commit
    /// does, then watch the remote catch up.
    #[dialog_common::test]
    async fn it_auto_syncs_a_local_write_to_the_remote(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        // Banner ⇒ the home space mounted, and with it the
        // background controller for its branches.
        driver.query(By::Css(".space-banner-title")).first().await?;

        // Commit a fact locally. The worker's evaluate route defaults
        // to transact=true, so this lands a commit on `main` — local
        // is now ahead of the auto-configured `origin` remote.
        let commit = driver
            .execute(
                r#"
                const body = `attribute!: &auto-sync-probe
  the:         test.tonk/auto-sync-probe
  as:          text
  cardinality: one
  description: marker asserted by the auto-sync test
`;
                const response = await fetch('/api/repository/home/branch/main/evaluate', {
                    method: 'POST',
                    headers: { 'content-type': 'application/yaml' },
                    body,
                });
                return response.ok;
                "#,
                vec![],
            )
            .await?;
        assert_eq!(
            commit.json().as_bool(),
            Some(true),
            "local commit should succeed"
        );

        // Fire the same signal a real editor commit dispatches — no
        // button is pressed. The controller debounces, then syncs
        // every upstream branch of the active repository.
        driver
            .execute(
                &format!(
                    "window.dispatchEvent(new CustomEvent('{}'));",
                    crate::sync_controller::COMMITTED_EVENT
                ),
                vec![],
            )
            .await?;

        // Poll the remote until its head matches the local head. The
        // `tree` reference serializes as a byte array, so compare the
        // JSON encodings rather than identity.
        let caught_up = driver
            .execute(
                r#"
                const localResp = await fetch('/api/repository/home');
                const localInfo = await localResp.json();
                const localTree = JSON.stringify(localInfo.branch.main.revision.tree);

                const deadline = Date.now() + 15000;
                while (Date.now() < deadline) {
                    const r = await fetch('/api/inspect/repository/home/remote/origin/branch/main');
                    const s = await r.json();
                    if (s.success && s.revision && JSON.stringify(s.revision.tree) === localTree) {
                        return true;
                    }
                    await new Promise(res => setTimeout(res, 500));
                }
                return false;
                "#,
                vec![],
            )
            .await?;
        assert_eq!(
            caught_up.json().as_bool(),
            Some(true),
            "remote should catch up to the local head via background sync, no button press"
        );

        driver.quit().await?;
        Ok(())
    }

    /// With auto-sync paused for a repository, a local write does
    /// *not* reach the remote on its own — only the manual buttons
    /// act. Exercises the per-repository pause preference, honored by
    /// the controller on every sweep.
    #[dialog_common::test]
    async fn it_does_not_auto_sync_a_paused_repository(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        driver.query(By::Css(".space-banner-title")).first().await?;

        // Pause auto-sync for `home` before writing anything.
        driver
            .execute(
                "localStorage.setItem('tonk:auto-sync:home', 'off'); return true;",
                vec![],
            )
            .await?;

        // Commit locally — `home/main` is now ahead of `origin`.
        let commit = driver
            .execute(
                r#"
                const body = `attribute!: &paused-probe
  the:         test.tonk/paused-probe
  as:          text
  cardinality: one
  description: marker asserted by the paused auto-sync test
`;
                const response = await fetch('/api/repository/home/branch/main/evaluate', {
                    method: 'POST',
                    headers: { 'content-type': 'application/yaml' },
                    body,
                });
                return response.ok;
                "#,
                vec![],
            )
            .await?;
        assert_eq!(commit.json().as_bool(), Some(true), "local commit succeeds");

        // Fire the commit signal, then wait well past the controller's
        // debounce. A paused repo must leave the remote untouched.
        driver
            .execute(
                &format!(
                    "window.dispatchEvent(new CustomEvent('{}'));",
                    crate::sync_controller::COMMITTED_EVENT
                ),
                vec![],
            )
            .await?;

        let synced = driver
            .execute(
                r#"
                const localResp = await fetch('/api/repository/home');
                const localInfo = await localResp.json();
                const localTree = JSON.stringify(localInfo.branch.main.revision.tree);

                // Generous margin past the 1s commit debounce.
                await new Promise(res => setTimeout(res, 5000));

                const r = await fetch('/api/inspect/repository/home/remote/origin/branch/main');
                const s = await r.json();
                return !!(s.success && s.revision && JSON.stringify(s.revision.tree) === localTree);
                "#,
                vec![],
            )
            .await?;
        assert_eq!(
            synced.json().as_bool(),
            Some(false),
            "a paused repository must not auto-sync the local write to the remote"
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

        // Wait for the home space's bare display route to mount.
        // The `<tonk-code>` editor mounts within the display once
        // the space is ready, without us toggling anything.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

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

    /// End-to-end exercise of `<tonk-display>`'s event-handling
    /// pipeline. Bootstrap a counter concept + view + an increment
    /// rule on the home repo via `/evaluate`, navigate to the
    /// display route, click the rendered `+` button, and verify
    /// the rendered count goes up. Repeat twice to catch any
    /// listener-after-rerender regression.
    ///
    /// Covers the full chain inside the browser:
    /// preprocess (`onclick=increment` → `data-onclick="increment"`),
    /// per-event delegation listener install on the host, descriptor
    /// resolve, event-path projection (`event.target.dataset.counter`),
    /// transient assertion POST to `/transact`, worker fixpoint, and
    /// re-rendered count.
    #[dialog_common::test]
    async fn it_drives_a_counter_via_event_handlers(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;
        driver
            .set_script_timeout(std::time::Duration::from_secs(30))
            .await?;

        // Wait for the home space's bare display route to be ready
        // before bootstrapping.
        driver
            .query(By::Css("tonk-repository.display-route"))
            .first()
            .await?;

        // Bootstrap the schema + a seeded counter, all in one
        // evaluate. The view's `display` template names `counter`
        // as the data-counter source (so the click handler sees
        // the rendered entity), uses `onclick=increment` to bind,
        // and renders `{count}` so we can read the value back.
        //
        // `counter-name` is a bookmark we navigate to from the
        // display route.
        let setup = r#"
attribute!: &view-model
  description: "view model"
  the:         xyz.tonk.view/model
  as:          entity
  cardinality: one

attribute!: &view-display
  description: "view display template"
  the:         xyz.tonk.view/display
  as:          text
  cardinality: one

concept!: &view
  description: "A rendered view of an entity"
  with:
    model:   view-model
    display: view-display

concept!: &counter
  with:
    count:
      the:         xyz.tonk.counter/count
      as:          unsigned-integer
      cardinality: one
      description: "current count"

concept!: &increment
  transient:
  with:
    counter:
      the:         dom.event.target.dataset/counter
      as:          entity
      cardinality: one
      description: "the counter to bump"

rule!:
  assert!: counter
  when:
    - assert: increment
      where: { counter: ?this }
    - assert: counter
      where: { this: ?this, count: ?m }
    - assert: math/sum
      where: { of: 1, with: ?m, is: ?count }

view!: &counter-view
  model:   counter
  display: "<div><button onclick=increment data-counter={this}>+</button><span class=\"value\">{count}</span></div>"

counter!: &my-counter
  count: 0
"#;
        let setup_result = driver
            .execute(
                r#"
                const body = arguments[0];
                const response = await fetch('/api/repository/home/branch/main/evaluate', {
                    method: 'POST',
                    headers: { 'content-type': 'application/yaml' },
                    body,
                });
                if (!response.ok) {
                    const text = await response.text();
                    throw new Error('setup evaluate failed: ' + response.status + ' ' + text);
                }
                return await response.json();
                "#,
                vec![serde_json::Value::String(setup.to_owned())],
            )
            .await?;
        // The evaluate response carries a `commits.claims` count;
        // a zero count would mean nothing actually wrote. We don't
        // assert a specific number (it depends on attribute /
        // concept structural facts) but it should be non-trivial.
        let claims = setup_result
            .json()
            .pointer("/commits/claims")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        assert!(
            claims > 5,
            "expected the setup evaluate to write several claims, got {claims}: {:?}",
            setup_result.json()
        );

        // Navigate to the display route for the named counter. The
        // `{entity}@{model}!{view}` subject encoding carries the
        // view and model into the path: `my-counter@counter!counter-view`.
        // The route resolves the bookmark `my-counter` to its
        // entity URI via the built-in Name index.
        driver
            .goto(&format!(
                "{}space/home/my-counter@counter!counter-view",
                env.tonk_web
            ))
            .await?;

        // Poll the rendered `.value` span until it shows `0` —
        // confirms the display mounted and the entity subscription
        // landed a frame.
        let initial = poll_text(&driver, ".value", "0", 10_000).await?;
        assert_eq!(
            initial, "0",
            "expected the seeded count to render as 0, got {initial}"
        );

        // Click the `+` button. Read back the value once the
        // worker's induce loop + subscription re-fire have updated
        // the rendered span.
        driver
            .query(By::Css("button[data-onclick=\"increment\"]"))
            .first()
            .await?
            .click()
            .await?;
        let after_first = poll_text(&driver, ".value", "1", 10_000).await?;
        assert_eq!(
            after_first, "1",
            "expected count to increment to 1 after first click"
        );

        // Click again. A second click after the first proves the
        // listener survived the incremental re-render driven by the
        // entity subscription — the renderer patches `.value`'s
        // text node, but the delegation listener lives on the
        // host so the binding still routes.
        driver
            .query(By::Css("button[data-onclick=\"increment\"]"))
            .first()
            .await?
            .click()
            .await?;
        let after_second = poll_text(&driver, ".value", "2", 10_000).await?;
        assert_eq!(
            after_second, "2",
            "expected count to increment to 2 after second click"
        );

        driver.quit().await?;
        Ok(())
    }

    /// Poll a CSS selector's `.text()` until it equals
    /// `expected`, or `deadline_ms` elapses. Returns whatever
    /// the latest text was — `Ok(value)` on a successful match,
    /// `Ok(latest)` on timeout (the caller asserts).
    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(feature = "integration-tests"), allow(dead_code))]
    async fn poll_text(
        driver: &WebDriver,
        selector: &str,
        expected: &str,
        deadline_ms: u64,
    ) -> Result<String> {
        let start = std::time::Instant::now();
        let mut latest = String::new();
        while start.elapsed() < std::time::Duration::from_millis(deadline_ms) {
            if let Ok(el) = driver.query(By::Css(selector)).first().await
                && let Ok(text) = el.text().await
            {
                latest = text;
                if latest == expected {
                    return Ok(latest);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(latest)
    }
}
