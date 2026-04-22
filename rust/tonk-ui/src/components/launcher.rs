use crate::api;
use crate::components::{
    NewRepoVisible, RepoListResource, Status, TonkJoin, TonkNewRepo, TonkSpace, TonkToolbar,
};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view: toolbar column + routed workspace + overlays.
///
/// Owns the repo-list resource and the new-repo overlay visibility
/// signal, and provides both via context so the toolbar (reader +
/// writer) and overlay components can share one source of truth.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");

    // The list fetch is gated on `Status::Ready` — the worker's
    // `GET /api/repositories` opens home, so it 500s until
    // TonkShell's init has PUT home first. Tracking status here
    // means the resource re-runs when init completes. Before then
    // it resolves to an empty list, which is invisible anyway
    // since the toolbar is translated off-screen until Ready.
    let repos = LocalResource::new(move || {
        let ready = matches!(status.get(), Status::Ready);
        async move {
            if !ready {
                return Ok::<_, String>(Vec::new());
            }
            api::list_repositories().await.map_err(|e| format!("{e}"))
        }
    });
    provide_context(RepoListResource(repos));

    let new_repo_visible = RwSignal::new(false);
    provide_context(NewRepoVisible(new_repo_visible));

    view! {
        <Router>
            <section class="launcher">
                <TonkToolbar />
                <Routes fallback=move || view!{ <section class="404">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("space/:space?") view=TonkSpace />
                    <Route path=path!("join") view=TonkJoin />
                </Routes>
                <TonkNewRepo />
            </section>
        </Router>
    }
}

#[cfg(all(
    test,
    not(any(target_arch = "wasm32", feature = "web-integration-tests"))
))]
mod integration_tests {
    #![allow(unexpected_cfgs)]

    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use crate::helpers::TestEnvironment;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use anyhow::Result;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use thirtyfour::prelude::*;

    #[dialog_common::test]
    async fn it_navigates_to_the_default_space(test_environment: TestEnvironment) -> Result<()> {
        let driver = test_environment.driver().await?;

        // Wait for the shell's `PUT /api/repository/home` to complete
        // and the redirect to land on `/space/home`; `.repository`
        // appears when the fetch succeeds.
        let repository = driver.query(By::Css("pre.repository")).first().await?;

        // The server returns the repo's DID under `"subject"`, so
        // the rendered JSON should at least mention it.
        let text = repository.text().await?;
        assert!(
            text.contains("\"subject\""),
            "expected repository JSON to include a subject field, got: {text}",
        );
        assert!(
            text.contains("did:key:"),
            "expected repository JSON to include a did:key value, got: {text}",
        );

        driver.quit().await?;

        Ok(())
    }
}
