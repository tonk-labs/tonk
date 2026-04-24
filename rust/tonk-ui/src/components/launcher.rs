use crate::components::{TonkCreateSpace, TonkProfile, TonkSpace, TonkToolbar};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view. `<wa-page>` provides the adaptive shell:
/// navigation sits in its own column on desktop and collapses into a
/// drawer (with a hamburger toggle) below the mobile breakpoint.
///
/// The create-space dialog is mounted here once (outside the
/// `<Routes>`) so it survives navigation and can be triggered from
/// any screen via the shared [`CreateSpaceOpen`] signal.
///
/// [`CreateSpaceOpen`]: super::CreateSpaceOpen
#[component]
pub fn TonkLauncher() -> impl IntoView {
    view! {
        <Router>
            <wa-page>
                <TonkToolbar />
                <Routes fallback=move || view!{ <section class="not-found">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("space/:space?") view=TonkSpace />
                    <Route path=path!("profile") view=TonkProfile />
                </Routes>
            </wa-page>
            <TonkCreateSpace />
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
        // and the redirect to land on `/space/home`. `.space-banner-title`
        // is only rendered after `repository_view` resolves, so it
        // doubles as the "page is ready" signal. Its `title`
        // attribute carries the repository's subject DID.
        let title = driver.query(By::Css(".space-banner-title")).first().await?;
        assert_eq!(title.text().await?, "home");
        let subject = title.attr("title").await?.unwrap_or_default();
        assert!(
            subject.starts_with("did:key:"),
            "expected banner title attribute to be a did:key, got: {subject}",
        );

        // The meta branch is bootstrapped by the worker on create
        // and the `main` branch is declared by `api::init`, so both
        // should appear as cards.
        let name_elements = driver
            .query(By::Css(".branch-card .branch-card-name"))
            .all_from_selector()
            .await?;
        let mut branch_names = Vec::with_capacity(name_elements.len());
        for element in name_elements {
            branch_names.push(element.text().await?);
        }
        assert!(
            branch_names.iter().any(|n| n == "main"),
            "expected a `main` branch card, got: {branch_names:?}",
        );
        assert!(
            branch_names.iter().any(|n| n == "meta"),
            "expected a `meta` branch card, got: {branch_names:?}",
        );

        // `api::init` wires up an `origin` remote; at least one
        // remote tile should render.
        assert!(
            driver.query(By::Css(".remote-card")).exists().await?,
            "expected at least one remote card to render",
        );

        driver.quit().await?;

        Ok(())
    }
}
