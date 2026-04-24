use crate::components::{TonkSpace, TonkToolbar};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view. `<wa-page>` provides the adaptive shell:
/// navigation sits in its own column on desktop and collapses into a
/// drawer (with a hamburger toggle) below the mobile breakpoint.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    view! {
        <Router>
            <wa-page>
                <TonkToolbar />
                <Routes fallback=move || view!{ <section>"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("space/:space?") view=TonkSpace />
                </Routes>
            </wa-page>
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
