use crate::components::{TonkEmpty, TonkJoin, TonkRepo, TonkSidebar, TonkSpace, TonkToolbar};
use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

/// Main launcher view: toolbar, persistent sidebar, and the routed view.
#[component]
pub fn TonkLauncher() -> impl IntoView {
    view! {
        <Router>
            <section class="launcher">
                <TonkToolbar />
                <TonkSidebar />
                <Routes fallback=move || view!{ <section class="404">"Nothing here ¯\\_(ツ)_/¯"</section> }>
                    <Route path=path!("") view=TonkEmpty />
                    <Route path=path!("join") view=TonkJoin />
                    <Route path=path!("repo/:did?") view=TonkRepo />
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
    use dialog_credentials::ed25519::Ed25519Signer;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use dialog_ucan_core::{
        DelegationBuilder, DelegationChain, principal::Principal, subject::Subject as UcanSubject,
    };
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use thirtyfour::prelude::*;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use tonk_invite::{Invite, InviteAudience};

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

    /// Navigate to `/join?...#...` with a freshly-minted open invite and
    /// assert the service worker claims it end-to-end.
    ///
    /// The test access service boots as part of [`TestEnvironment`] and is
    /// proxied at `/ucan/`. The claim endpoint does not yet call out to
    /// the access service with the embedded `remote_url` (see the comment
    /// on `claim_invite` in `tonk-worker`), but wiring the URL realistically
    /// now guards against regressions when that follow-up lands.
    #[dialog_common::test]
    async fn it_claims_an_open_invite_via_the_join_route(
        test_environment: TestEnvironment,
    ) -> Result<()> {
        const SUBJECT_SEED: [u8; 32] = [11u8; 32];
        const ISSUER_SEED: [u8; 32] = [12u8; 32];
        const EPHEMERAL_SEED: [u8; 32] = [13u8; 32];

        let subject_did = Ed25519Signer::import(&SUBJECT_SEED).await?.did();
        let ephemeral_did = Ed25519Signer::import(&EPHEMERAL_SEED).await?.did();
        let issuer = Ed25519Signer::import(&ISSUER_SEED).await?;

        let delegation = DelegationBuilder::new()
            .issuer(issuer)
            .audience(&ephemeral_did)
            .subject(UcanSubject::Specific(subject_did.clone()))
            .command(vec![])
            .try_build()
            .await?;
        let chain = DelegationChain::new(delegation);

        let remote_url = test_environment.tonk_web.join("/ucan/")?;
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            Some(remote_url),
        )?;
        let join_base = format!("{}join", test_environment.tonk_web);
        let invite_url = invite.to_url(&join_base)?;

        let driver = test_environment.driver().await?;
        driver.goto(&invite_url).await?;

        // Block until the claim resource resolves to either success or error.
        let status = driver
            .query(By::Css(".join .status.ok, .join .status.error"))
            .first()
            .await?;
        let status_class = status.attr("class").await?.unwrap_or_default();
        assert!(
            status_class.contains("ok"),
            "expected successful claim, got status classes {status_class:?}"
        );

        let did = driver.query(By::Css(".join code.did")).first().await?;
        assert_eq!(did.text().await?, subject_did.to_string());

        driver.quit().await?;
        Ok(())
    }
}
