use crate::components::{TonkSpace, TonkToolbar};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view that combines the toolbar and workspace.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    view! {
        <Router>
            <section class="launcher">
                <TonkToolbar />
                <Routes fallback=move || view!{ <section class="404">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("space/:did?") view=TonkSpace />
                </Routes>
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

        let _launcher = driver
            .query(By::Css(".launcher"))
            .with_text("Nothing here ¯\\_(ツ)_/¯")
            .first()
            .await?;

        let space = driver.query(By::Css(".space")).first().await?;

        assert!(space.text().await?.starts_with("did:key:"));

        driver.quit().await?;

        Ok(())
    }
}
